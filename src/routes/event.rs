use crate::state::session::Session;
use crate::state::state::{GoogleEvent, SseEvent};
use crate::filestreamer::FileStreamer;
use crate::{google, AppState};
use crate::logs::*;

use std::collections::HashMap;
use std::io::{Seek, SeekFrom};
use std::fs;

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use futures::future;
use reqwest::ClientBuilder;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resp {
  next_sync_token: String,
  items: Vec<Item>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DeletedEvent {
  id: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum Item {
  Event(GoogleEvent),
  Deleted(DeletedEvent),
}

#[actix_web::post("/webhook/v11")]
pub async fn index(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
  let app_state = state.read().await;
  let id = req.headers().get("x-goog-resource-id").and_then(|id| id.to_str().inspect_err(|err| error!("Error: {}", err)).ok());
  let id = match id {
    Some(id) => id,
    None => {
      error!("Error while getting id: not found");
      return HttpResponse::BadRequest().finish();
    }
  };
  
  let webhook = match app_state.calendar_webhooks.iter().find(|(_, webhook)| webhook.resource_id == id) {
    Some(webhook) => webhook,
    None => {
      error!("Error while getting webhook: not found");
      return HttpResponse::BadRequest().finish();
    }
  };
  
  let auth = app_state.users.get(webhook.0).unwrap().read().await.access_token.clone();
  let sync_token = webhook.1.sync_token.clone();
  let user = webhook.0.clone();
  
  let client = ClientBuilder::new()
    .danger_accept_invalid_certs(true)
    .build()
    .unwrap();

  drop(app_state);
  let google_req = client.get("https://www.googleapis.com/calendar/v3/calendars/primary/events")
    .bearer_auth(auth)
    .query(&[("syncToken", sync_token.as_str()), ("maxResults", "2500")])
    .send()
    .await;

  let google_req = match google_req {
    Ok(google_req) => google_req,
    Err(err) => {
      error!("Error while getting events: {}", err);
      return HttpResponse::InternalServerError().finish();
    }
  };

  let google_req = match google_req.json::<Resp>().await {
    Ok(google_req) => google_req,
    Err(err) => {
      error!("Error while parsing events: {}", err);
      return HttpResponse::InternalServerError().finish();
    }
  };

  let mut app_state = state.write().await;
  let path = format!("{}events/{}.json", app_state.path, user);

  let webhook = app_state.calendar_webhooks.get_mut(&user).unwrap();
  webhook.sync_token = google_req.next_sync_token;

  if google_req.items.len() == 0 {
    warning!("No events from the google calendar");
    return HttpResponse::NoContent().finish();
  }

  info!("Got {} events from the google calendar", google_req.items.len());
  let mut events = match fs::File::open(&path) {
    Ok(file) => {
      let mut file = file;
      file.seek(SeekFrom::Start(0)).unwrap();
      serde_json::from_reader::<_, Vec<GoogleEvent>>(file).unwrap()
    },
    Err(err) => {
      error!("Error while opening the file: {}", err);
      Vec::new()
    }
  };

  let user_email = app_state.users.get(&user).unwrap().read().await.user_info.email.clone();

  for event in google_req.items.into_iter() {
    match event {
      Item::Event(event) => {
        match events.iter_mut().find(|e| e.id == event.id) {
          Some(target) => {
            app_state.broadcast_to(SseEvent::EventUpdated(&event), &user).await;
            let is_session = app_state.sessions.iter().any(|s| s.calendar_ids.get(&user_email).is_some_and(|id| id == &event.id));
            if (target.start != event.start || target.end != event.end) && is_session {
              let session = app_state.sessions.iter_mut().find(|s| s.calendar_ids.get(&user_email).is_some_and(|id| id == &event.id)).unwrap();
              session.start = event.start.into_timestamp();
              session.end = event.end.into_timestamp();
              session.last_updated = Utc::now().timestamp() as u64;
              session.write();
              
              let session_clone = session.clone();
              app_state.broadcast(SseEvent::SessionUpdated(&session_clone)).await;
            }

            info!("Updated event {} in the local state", event.id);
            *target = event;
          },
          None => {
            app_state.broadcast_to(SseEvent::EventAdded(&event), &user).await;
            info!("Added event {} to the local state", event.id);
            let is_already_session = app_state.sessions.iter().any(|s| s.calendar_ids.get(&user_email).is_some_and(|id| id == &event.id));
            if let Some(summary) = &event.summary && !is_already_session {
              let patient = app_state.patients.iter().find(|p| &p.name == summary);
              if let Some(patient) = patient {
                let session = Session {
                  uuid: Uuid::new_v4().to_string(),
                  patient_uuid: patient.uuid.clone(),
                  start: event.start.into_timestamp(),
                  end: event.end.into_timestamp(),
                  paid: 0.0,
                  emotions: Vec::new(),
                  timeline: HashMap::new(),
                  created_at: Utc::now().timestamp() as u64,
                  last_updated: Utc::now().timestamp() as u64,
                  calendar_ids: HashMap::from([(user_email.clone(), event.id.to_owned())]),
                };

                session.write();
                app_state.broadcast(SseEvent::SessionAdded(&session)).await;
                app_state.sessions.push(session);
              }
            }

            events.push(event);
          },
        };
      },
      Item::Deleted(event) => {
        events.retain(|e| e.id != event.id);
        let session = app_state.sessions.iter().find(|s| s.calendar_ids.get(&user_email).is_some_and(|id| id == &event.id));
        if let Some(session) = session {
          session.delete();
          app_state.broadcast(SseEvent::SessionRemoved(&session.uuid)).await;
          
          let session_uuid = session.uuid.clone();
          app_state.sessions.retain(|s| s.uuid != session_uuid);
        }

        app_state.broadcast_to(SseEvent::EventRemoved(&event.id), &user).await;
        info!("Deleted event {} from the local state", event.id);
      },
    }
  }

  app_state.write();
  fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();
  HttpResponse::NoContent().finish()
}

#[actix_web::get("/events")]
pub async fn get_events(req: HttpRequest, state: web::Data<AppState>) -> Result<HttpResponse, actix_web::Error> {
  let app_state = state.read().await;
  let user = app_state.auth_token(req)?;

  let file = match fs::File::open(format!("{}events/{}.json", app_state.path, user)) {
    Ok(file) => file,
    Err(_) => {
      return Ok(HttpResponse::Ok().body("[]"));
    }
  };

  let streamer = FileStreamer(file);
  Ok(HttpResponse::Ok().streaming(streamer))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewEvent {
  start: u64,
  end: u64,
  patient: Option<String>,
  color_id: Option<String>,
  summary: String,
}

#[actix_web::post("/event")]
pub async fn create_event(req: HttpRequest, state: web::Data<AppState>, data: web::Json<NewEvent>) -> Result<HttpResponse, actix_web::Error> {
  let mut app_state = state.write().await;
  let user = app_state.auth_token(req)?;

  let data = data.into_inner();
  let event = google::RawCalendarEvent {
    start: data.start,
    end: data.end,
    description: None,
    summary: data.summary,
    uuid: String::new(),
    colorId: data.color_id,
  };
  
  let user = app_state.users.get(&user).unwrap().read().await;

  let event_id = match google::add_event(&user.access_token, &event).await {
    Ok(id) => id,
    Err(err) => {
      error!("Failed to edit event for user {}: {}", user.user_info.email, err);
      return Ok(HttpResponse::InternalServerError().body("Failed to add event"));
    },
  };

  let patient_uuid = match data.patient {
    Some(uuid) => uuid,
    None => return Ok(HttpResponse::Ok().finish()),
  };

  if !app_state.patients.iter().any(|p| p.uuid == patient_uuid) {
    return Ok(HttpResponse::BadRequest().body("Patient not found"));
  }

  let now = Utc::now().timestamp() as u64;
  let calendar_ids = HashMap::from([(user.user_info.email.clone(), event_id)]);

  let session = Session {
    uuid: Uuid::new_v4().to_string(),
    patient_uuid,
    start: data.start,
    end: data.end,
    paid: 0.0,
    emotions: Vec::new(),
    timeline: HashMap::new(),
    created_at: now,
    last_updated: now,
    calendar_ids,
  };

  drop(user);
  session.write();

  let payload = SseEvent::SessionAdded(&session);
  app_state.broadcast(payload).await;
  
  app_state.sessions.push(session);
  Ok(HttpResponse::Ok().finish())
}

#[derive(Debug, Deserialize)]
struct DeleteBody {
  id: String,
  kind: u8
}

#[actix_web::delete("/event")]
pub async fn delete_event(req: HttpRequest, state: web::Data<AppState>, body: web::Json<DeleteBody>) -> Result<HttpResponse, actix_web::Error> {  
  if !(2..=4).contains(&body.kind) {
    return Err(actix_web::error::ErrorBadRequest("Invalid kind"));
  }
  
  let mut app_state = state.write().await;
  let user = app_state.check_auth(req)?;
  
  if body.kind == 2 || body.kind == 4 {
    let session = match app_state.sessions.iter().find(|s| s.uuid == body.id) {
      Some(session) => session,
      None => return Ok(HttpResponse::NotFound().finish()),
    };

    let users = future::join_all(app_state.users.values().map(|u| u.read())).await;
    for (email, id) in session.calendar_ids.iter() {
      let user = users.iter().find(|u| &u.user_info.email == email).unwrap();
      match google::delete_event(&user.access_token, &id).await {
        Ok(_) => {},
        Err(err) => { error!("Failed to delete event for user {}: {}", user.user_info.email, err); },
      };
    }

    session.delete();
    app_state.broadcast(SseEvent::SessionRemoved(&body.id)).await;

    drop(users);
    app_state.sessions.retain(|s| s.uuid != body.id);
  }

  if body.kind == 3 {
    let user = user.read().await;
    if let Err(e) = google::delete_event(&user.access_token, &body.id).await {
      error!("Failed to delete event for user {}: {}", user.user_info.email, e);
      return Ok(HttpResponse::InternalServerError().finish());
    }
  }

  Ok(HttpResponse::Ok().finish())
}

#[derive(Debug, Deserialize)]
struct EditBody {
  start: u64,
  end: u64,
  color_id: Option<String>,
  summary: String,
}

#[actix_web::put("/event/{id}")]
pub async fn edit_event(req: HttpRequest, state: web::Data<AppState>, body: web::Json<EditBody>, id: web::Path<String>) -> Result<HttpResponse, actix_web::Error> {
  let app_state = state.read().await;
  let user = app_state.check_auth(req)?;

  let body = body.into_inner();
  let user = user.read().await;

  let event = google::EditEvent {
    start: body.start,
    end: body.end,
    description: None,
    summary: body.summary,
    id: id.to_string(),
    colorId: body.color_id,
  };

  if let Err(e) = google::edit_event(&user.access_token, &event).await {
    error!("Failed to edit event for user {}: {}", user.user_info.email, e);
    return Ok(HttpResponse::InternalServerError().finish());
  }

  Ok(HttpResponse::Ok().finish())
}
