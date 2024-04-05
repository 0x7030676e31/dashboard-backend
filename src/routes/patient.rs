use crate::google;
use crate::state::state::{SseEvent, DrainWith};
use crate::state::patient;
use crate::AppState;
use crate::logs::*;

use std::collections::HashMap;

use actix_web::{delete, post, patch, web, HttpResponse, HttpRequest};
use serde::Deserialize;
use uuid::Uuid;
use futures_util::future;

#[derive(Deserialize)]
struct NewPatient {
  name: String,
  address: String,
  age: Option<u8>,
}

#[post("/")]
pub async fn create_patient(req: HttpRequest, state: web::Data<AppState>, new_patient: web::Json<NewPatient>) -> Result<HttpResponse, actix_web::Error> {
  let mut state = state.write().await;
  state.auth_token(req)?;

  let name = new_patient.name.to_lowercase();
  if state.patients.iter().any(|patient| patient.name.to_lowercase() == name) {
    return Ok(HttpResponse::Conflict().body("Patient already exists"));
  }

  let uuid = Uuid::new_v4();
  let patient = patient::Patient {
    uuid: uuid.to_string(),
    description: String::new(),
    age: new_patient.age,
    name: new_patient.name.clone(),
    address: new_patient.address.clone(),
    profile_picture: None,
    created_at: chrono::Utc::now().timestamp() as u64,
    last_updated: chrono::Utc::now().timestamp() as u64,
  };

  patient.write();
  state.broadcast(SseEvent::PatientAdded(&patient)).await;
  state.patients.push(patient);
  
  info!("Created patient {}", new_patient.name);
  Ok(HttpResponse::Ok().body(uuid.to_string()))
}

#[derive(Deserialize)]
struct UpdatePatient {
  name: Option<String>,
  address: Option<String>,
  age: Option<u8>,
  description: Option<String>,
}

#[patch("/{uuid}")]
pub async fn update_patient(req: HttpRequest, state: web::Data<AppState>, uuid: web::Path<String>, update_patient: web::Json<UpdatePatient>) -> Result<HttpResponse, actix_web::Error> {
  let mut app_state = state.write().await;
  app_state.auth_token(req)?;

  let uuid = uuid.into_inner();
  let name = update_patient.name.as_ref().map(|name| name.to_lowercase());
  if update_patient.name.is_some() && app_state.patients.iter().any(|patient| patient.uuid != uuid && &patient.name.to_lowercase() == name.as_ref().unwrap()) {
    return Ok(HttpResponse::Conflict().body("Patient already exists"));
  }

  let patient = match app_state.patients.iter_mut().find(|patient| patient.uuid == uuid) {
    Some(patient) => patient,
    None => return Ok(HttpResponse::NotFound().body("Patient not found")),
  };

  let UpdatePatient { name, address, age, description } = update_patient.into_inner();

  if let Some(address) = address {
    patient.address = address;
  }

  if let Some(age) = age {
    patient.age = if age == 0 { None } else { Some(age) };
  }

  let mut do_update = false;
  if let Some(description) = description && description != patient.description {
    patient.description = description;
    do_update = true;
  }

  if let Some(name) = name && name != patient.name {
    patient.name = name;
    do_update = true;
  }

  if do_update {
    let desc = patient.description.clone();
    let patient_name = patient.name.clone();
    let summary = format!("S. {}", patient.name);
    let state2 = state.clone();
    tokio::spawn(async move {
      let state = state2.read().await;

      let sessions = state.sessions.iter().filter(|session| session.patient_uuid == uuid);
      let mut batches: HashMap<String, Vec<google::EditEvent>> = HashMap::new();
      
      for session in sessions {
        session.calendar_ids.iter().for_each(|(mail, id)| {
          batches.entry(mail.clone()).or_default().push(google::EditEvent {
            start: session.start,
            end: session.end,
            description: Some(desc.clone()),
            summary: summary.clone(),
            id: id.to_owned(),
            colorId: None,
          });
        });
      }

      let users = state.users.values().map(|u| u.read());
      let users = future::join_all(users).await;

      for user in users {
        let entries = match batches.get(&user.user_info.email) {
          Some(entries) => entries,
          None => continue,
        };

        if let Err(err) = google::edit_events(&user.access_token, &entries[..]).await {
          error!("Failed to edit events for user {}: {}", user.user_info.email, err);
        }
      }

      info!("Edited {} events for patient {}", batches.len(), patient_name);
    });
  }

  patient.last_updated = chrono::Utc::now().timestamp() as u64;
  patient.write();

  let patient = patient.clone();
  app_state.broadcast(SseEvent::PatientUpdated(&patient)).await;
  
  info!("Updated patient {}", patient.name);
  Ok(HttpResponse::Ok().body("Patient updated"))
}

#[delete("/{uuid}")]
pub async fn delete_patient(req: HttpRequest, state: web::Data<AppState>, uuid: web::Path<String>) -> Result<HttpResponse, actix_web::Error> {
  let mut app_state = state.write().await;
  app_state.auth_token(req)?;

  let uuid = uuid.into_inner();
  let patient = match app_state.patients.iter().position(|patient| patient.uuid == uuid) {
    Some(index) => app_state.patients.remove(index),
    None => return Ok(HttpResponse::NotFound().body("Patient not found")),
  };

  let mut event_ids: HashMap<String, Vec<String>> = HashMap::new();
  let sessions = app_state.sessions.drain_with(|session| session.patient_uuid == uuid);
  for session in sessions {
    session.delete();
    session.calendar_ids.iter().for_each(|(mail, id)| {
      event_ids.entry(mail.clone()).or_default().push(id.clone());
    });
  }

  patient.delete();
  app_state.broadcast(SseEvent::PatientRemoved(&uuid)).await;
  

  let app_state = state.clone();
  let patient_name = patient.name.clone();
  tokio::spawn(async move {
    let app_state = app_state.write().await;
    let users = app_state.users.values().map(|u| u.read());
    let users = future::join_all(users).await;
    for user in users {
      let entries = match event_ids.get(&user.user_info.email) {
        Some(entries) => entries,
        None => continue,
      };

      if let Err(err) = google::delete_events(&user.access_token, &entries[..]).await {
        error!("Failed to delete events for user {}: {}", user.user_info.email, err);
      }
    }

    info!("Deleted events for patient {}", patient_name);
  });

  info!("Deleted patient {}", patient.name);
  Ok(HttpResponse::Ok().body("Patient deleted"))
}