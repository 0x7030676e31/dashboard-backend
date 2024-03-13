use crate::state::state::{GoogleEvent, SseEvent};
use crate::filestreamer::FileStreamer;
use crate::AppState;
use crate::logs::*;

use std::io::{Seek, SeekFrom};
use std::fs;

use actix_web::{web, HttpRequest, HttpResponse};
use reqwest::ClientBuilder;
use serde::Deserialize;

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

#[actix_web::post("/webhook/v6")]
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

  if google_req.items.len() > 1 {
    warning!("More than one event in the response");
  }

  if google_req.items.len() == 0 {
    return HttpResponse::NoContent().finish();
  }

  info!("Got {} events from the google calendar", google_req.items.len());
  match google_req.items[0].to_owned() {
    Item::Event(event) => {
      let mut file = fs::File::open(&path).map_err(|err| error!("Error while opening the file: {}", err)).unwrap();
      let mut events = serde_json::from_reader::<_, Vec<GoogleEvent>>(&file).unwrap();
      if !webhook.events.contains(&event.id) {
        events.push(event.clone());
        webhook.events.push(event.id.clone());

        app_state.broadcast_to(SseEvent::EventAdded(&event), &user).await;
      } else {
        let target = events.iter_mut().find(|e| e.id == event.id).unwrap();
        app_state.broadcast_to(SseEvent::EventUpdated(&event), &user).await;
        let id = event.id.clone();
        *target = event;

        info!("Updated event {} in the file {}", id, path);
      }

      file.seek(SeekFrom::Start(0)).unwrap();
      file.set_len(0).unwrap();
      serde_json::to_writer(&file, &events).unwrap();
    },
    Item::Deleted(event) => {
      webhook.events.retain(|e| e != &event.id);
      app_state.broadcast_to(SseEvent::EventRemoved(&event.id), &user).await;
      info!("Deleted event {} from the local state", event.id);
    }
  }

  app_state.write();
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