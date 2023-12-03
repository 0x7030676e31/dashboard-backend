use crate::state::session::SessionSocket;
use crate::state::session::Session;
use crate::AppState;
use crate::state::state::SseEvent;
use crate::logs::*;

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::delete;
use actix_web::{HttpRequest, web, Error, HttpResponse, get, post, patch};
use actix_web_actors::ws;
use serde::Deserialize;

#[derive(Deserialize)]
struct NewPatient {
  patient: String,
  time_start: u64,
  time_end: u64,
}

#[post("/")]
pub async fn create_session(req: HttpRequest, state: web::Data<AppState>, new_session: web::Json<NewPatient>) -> Result<HttpResponse, actix_web::Error> {
  let mut state = state.write().await;
  state.auth_token(req)?;

  if !state.patients.iter().any(|patient| patient.uuid == new_session.patient) {
    return Ok(HttpResponse::NotFound().body("Patient not found"));
  }

  let NewPatient { patient, time_start, time_end } = new_session.into_inner();
  info!("Created session for patient {}", patient);

  let uuid = uuid::Uuid::new_v4();
  let session = Session {
    uuid: uuid.to_string(),
    patient_uuid: patient,
    start: time_start,
    end: time_end,
    paid: 0.0,
    emotions: Vec::new(),
    timeline: HashMap::new(),
    created_at: chrono::Utc::now().timestamp() as u64,
    last_updated: chrono::Utc::now().timestamp() as u64,
  };
  
  session.write();
  state.broadcast(SseEvent::SessionAdded(&session)).await;
  state.sessions.push(session);

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
pub async fn update_session(req: HttpRequest, state: web::Data<AppState>, payload: web::Json<UpdateSession>, session: web::Path<String>) -> Result<HttpResponse, Error> {
  let mut state = state.write().await;
  state.auth_token(req)?;

  let UpdateSession { patient, time_start, time_end, paid } = payload.into_inner();
  if patient.is_some() && !state.patients.iter().any(|pt| patient.as_ref().is_some_and(|p| p == &pt.uuid)) {
    return Ok(HttpResponse::NotFound().body("Patient not found"));
  }

  let session = session.into_inner();
  let session = match state.sessions.iter_mut().find(|s| &s.uuid == &session) {
    Some(session) => session,
    None => return Ok(HttpResponse::NotFound().body("Not Found")),
  };

  if let Some(patient) = patient {
    session.patient_uuid = patient;
  }

  if let Some(time_start) = time_start {
    session.start = time_start;
  }

  if let Some(time_end) = time_end {
    session.end = time_end;
  }

  if let Some(paid) = paid {
    session.paid = paid;
  }

  session.last_updated = chrono::Utc::now().timestamp() as u64;
  session.write();

  let session = session.clone();
  state.broadcast(SseEvent::SessionUpdated(&session)).await;

  info!("Updated session {}", session.uuid);
  Ok(HttpResponse::Ok().body("Updated"))
}

#[delete("/{uuid}")]
pub async fn delete_session(req: HttpRequest, state: web::Data<AppState>, session: web::Path<String>) -> Result<HttpResponse, Error> {
  let mut state = state.write().await;
  state.auth_token(req)?;

  let uuid = session.into_inner();
  let session = match state.sessions.iter().find(|s| &s.uuid == &uuid) {
    Some(session) => session,
    None => return Ok(HttpResponse::NotFound().body("Not Found")),
  };

  session.delete();
  state.broadcast(SseEvent::SessionRemoved(&uuid)).await;
  state.sessions.retain(|s| s.uuid != uuid);

  info!("Deleted session {}", uuid);
  Ok(HttpResponse::Ok().body("Deleted"))
}

#[get("/{session}/stream")]
pub async fn stream(req: HttpRequest, state: web::Data<AppState>, payload: web::Payload, session: web::Path<String>) -> Result<HttpResponse, Error> {
  let session = session.into_inner();
  let app_state = state.clone();
  let app_state = app_state.read().await;
  
  if !app_state.sessions.iter().any(|s| &s.uuid == &session) {
    return Ok(HttpResponse::NotFound().body("Not Found"));
  }

  let socket = SessionSocket::new(Arc::clone(&*state.into_inner()), session);
  let resp = ws::start(socket, &req, payload)?;
  Ok(resp)
}
