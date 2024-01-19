use crate::state::session::{SessionSocket, Session, Emotion};
use crate::{AppState, google};
use crate::state::state::SseEvent;
use crate::logs::*;

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::delete;
use actix_web::{HttpRequest, web, Error, HttpResponse, get, post, patch};
use actix_web_actors::ws;
use futures_util::future;
use serde::Deserialize;

#[derive(Deserialize)]
struct NewPatient {
  patient: String,
  time_start: u64,
  time_end: u64,
}

#[post("/")]
pub async fn create_session(req: HttpRequest, state: web::Data<AppState>, new_session: web::Json<NewPatient>) -> Result<HttpResponse, actix_web::Error> {
  let mut app_state = state.write().await;
  app_state.auth_token(req)?;

  if !app_state.patients.iter().any(|patient| patient.uuid == new_session.patient) {
    return Ok(HttpResponse::NotFound().body("Patient not found"));
  }

  let NewPatient { patient, time_start, time_end } = new_session.into_inner();
  info!("Created session for patient {}", patient);

  let emotion_uuid = uuid::Uuid::new_v4();
  let uuid = uuid::Uuid::new_v4();

  let session = Session {
    uuid: uuid.to_string(),
    patient_uuid: patient,
    start: time_start,
    end: time_end,
    paid: 0.0,
    emotions: vec![Emotion {
      uuid: emotion_uuid.to_string(),
      id: None,
      kind: None,
      aquired_age: None,
      aquired_person: String::new(),
      created_at: chrono::Utc::now().timestamp() as u64,
    }],
    timeline: HashMap::new(),
    created_at: chrono::Utc::now().timestamp() as u64,
    last_updated: chrono::Utc::now().timestamp() as u64,
    calendar_ids: HashMap::new(),
  };

  let patient = app_state.patients.iter().find(|patient| patient.uuid == session.patient_uuid).unwrap();
  let raw_calendar_event = google::RawCalendarEvent {
    start: time_start,
    end: time_end,
    description: Some(patient.description.to_owned()),
    summary: format!("S. {}", if patient.name.is_empty() { "<Pacjent bez nazwy>" } else { patient.name.as_str() }),
    uuid: uuid.to_string(),
  };

  session.write();
  app_state.broadcast(SseEvent::SessionAdded(&session)).await;
  app_state.sessions.push(session);

  let app_state = state.clone();
  tokio::spawn(async move {
    let mut app_state = app_state.write().await;
    let mut ids = HashMap::new();
    
    for user in app_state.users.values() {      
      let user = user.read().await;
      if !user.settings.google_calendar_enabled {
        continue;
      }

      match google::add_event(&user.access_token, &raw_calendar_event).await {
        Ok(id) => { ids.insert(user.user_info.email.clone(), id); },
        Err(err) => { error!("Failed to add event for user {}: {}", user.user_info.email, err); },
      };
    }

    let session = app_state.sessions.iter_mut().find(|s| s.uuid == uuid.to_string()).unwrap();
    session.calendar_ids = ids;

    session.write();
    info!("Added events for session {}", uuid);
  });
  
  Ok(HttpResponse::Ok().body(uuid.to_string()))
}

#[derive(Deserialize)]
struct UpdateSession {
  patient: Option<String>,
  time_start: Option<u64>,
  time_end: Option<u64>,
  paid: Option<f32>,
}

#[patch("/{uuid}")]
pub async fn update_session(req: HttpRequest, state: web::Data<AppState>, payload: web::Json<UpdateSession>, session_uuid: web::Path<String>) -> Result<HttpResponse, Error> {
  let mut app_state = state.write().await;
  app_state.auth_token(req)?;

  let UpdateSession { patient, time_start, time_end, paid } = payload.into_inner();
  if patient.is_some() && !app_state.patients.iter().any(|pt| patient.as_ref().is_some_and(|p| p == &pt.uuid)) {
    return Ok(HttpResponse::NotFound().body("Patient not found"));
  }

  let session_uuid = session_uuid.into_inner();
  let session = match app_state.sessions.iter_mut().find(|s| s.uuid == session_uuid) {
    Some(session) => session,
    None => return Ok(HttpResponse::NotFound().body("Not Found")),
  };

  let mut do_update = false;
  if let Some(patient) = patient && patient != session.patient_uuid {
    session.patient_uuid = patient;
    do_update = true;
  }

  if let Some(time_start) = time_start && time_start != session.start {
    session.start = time_start;
    do_update = true;
  }

  if let Some(time_end) = time_end && time_end != session.end {
    session.end = time_end;
    do_update = true;
  }

  if do_update {
    let state2 = state.clone();
    let ids = session.calendar_ids.clone();
    let start = session.start;
    let end = session.end;
    let patient_uuid = session.patient_uuid.clone();
    tokio::spawn(async move {
      let state = state2.read().await;
    
      let patient = state.patients.iter().find(|p| p.uuid == patient_uuid).unwrap();
      let summary = format!("S. {}", patient.name);
    
      let users = state.users.values().map(|u| u.read());
      let users = future::join_all(users).await;

      for (email, id) in ids {
        let user = users.iter().find(|u| u.user_info.email == email).unwrap();
        let event_edit = google::EditEvent {
          start,
          end,
          description: Some(patient.description.to_owned()),
          summary: summary.clone(),
          id,
        };
        
        match google::edit_event(&user.access_token, &event_edit).await {
          Ok(_) => {},
          Err(err) => { error!("Failed to edit event for user {}: {}", user.user_info.email, err); },
        };
      }

      info!("Edited events for session {}", session_uuid);
    });
  }

  if let Some(paid) = paid {
    session.paid = paid;
  }

  session.last_updated = chrono::Utc::now().timestamp() as u64;
  session.write();

  let session = session.clone();
  app_state.broadcast(SseEvent::SessionUpdated(&session)).await;

  info!("Updated session {}", session.uuid);
  Ok(HttpResponse::Ok().body("Updated"))
}

#[delete("/{uuid}")]
pub async fn delete_session(req: HttpRequest, state: web::Data<AppState>, session: web::Path<String>) -> Result<HttpResponse, Error> {
  let mut app_state = state.write().await;
  app_state.auth_token(req)?;

  let uuid = session.into_inner();
  let session = match app_state.sessions.iter().find(|s| s.uuid == uuid) {
    Some(session) => session,
    None => return Ok(HttpResponse::NotFound().body("Not Found")),
  };

  let calendar_ids = session.calendar_ids.clone();
  let state = state.clone();
  let uuid2 = uuid.clone();
  tokio::spawn(async move {
    let state = state.write().await;
    let users = state.users.values().map(|u| u.read());
    let users = future::join_all(users).await;
    
    let len = calendar_ids.len();
    for (email, id) in calendar_ids {
      let user = users.iter().find(|u| u.user_info.email == email).unwrap();
      match google::delete_event(&user.access_token, &id).await {
        Ok(_) => {},
        Err(err) => { error!("Failed to delete event for user {}: {}", user.user_info.email, err); },
      };
    }

    info!("Deleted {} events for session {}", len, uuid2);
  });

  session.delete();
  app_state.broadcast(SseEvent::SessionRemoved(&uuid)).await;
  app_state.sessions.retain(|s| s.uuid != uuid);

  info!("Deleted session {}", uuid);
  Ok(HttpResponse::Ok().body("Deleted"))
}

#[get("/{session}/stream")]
pub async fn stream(req: HttpRequest, state: web::Data<AppState>, payload: web::Payload, session: web::Path<String>) -> Result<HttpResponse, Error> {
  let session = session.into_inner();
  let app_state = state.clone();
  let app_state = app_state.read().await;
  
  if !app_state.sessions.iter().any(|s| s.uuid == session) {
    return Ok(HttpResponse::NotFound().body("Not Found"));
  }

  let socket = SessionSocket::new(Arc::clone(&*state.into_inner()), session);
  let resp = ws::start(socket, &req, payload)?;
  Ok(resp)
}
