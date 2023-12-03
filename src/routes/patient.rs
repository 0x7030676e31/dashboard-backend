use crate::state::state::SseEvent;
use crate::state::patient;
use crate::AppState;
use crate::logs::*;

use actix_web::delete;
use actix_web::{post, patch, web, HttpResponse, HttpRequest};
use serde::Deserialize;
use uuid::Uuid;

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
  let mut state = state.write().await;
  state.auth_token(req)?;

  let uuid = uuid.into_inner();
  let name = update_patient.name.as_ref().map(|name| name.to_lowercase());
  if update_patient.name.is_some() && state.patients.iter().any(|patient| patient.uuid != uuid && &patient.name.to_lowercase() == name.as_ref().unwrap()) {
    return Ok(HttpResponse::Conflict().body("Patient already exists"));
  }

  let patient = match state.patients.iter_mut().find(|patient| patient.uuid == uuid.to_string()) {
    Some(patient) => patient,
    None => return Ok(HttpResponse::NotFound().body("Patient not found")),
  };

  let UpdatePatient { name, address, age, description } = update_patient.into_inner();

  if let Some(name) = name {
    patient.name = name;
  }

  if let Some(address) = address {
    patient.address = address;
  }

  if let Some(age) = age {
    patient.age = Some(age);
  }

  if let Some(description) = description {
    patient.description = description;
  }

  patient.last_updated = chrono::Utc::now().timestamp() as u64;
  patient.write();

  let patient = patient.clone();
  state.broadcast(SseEvent::PatientUpdated(&patient)).await;
  
  info!("Updated patient {}", patient.name);
  Ok(HttpResponse::Ok().body("Patient updated"))
}

#[delete("/{uuid}")]
pub async fn delete_patient(req: HttpRequest, state: web::Data<AppState>, uuid: web::Path<String>) -> Result<HttpResponse, actix_web::Error> {
  let mut state = state.write().await;
  state.auth_token(req)?;

  let uuid = uuid.into_inner();
  let patient = match state.patients.iter().position(|patient| patient.uuid == uuid.to_string()) {
    Some(index) => state.patients.remove(index),
    None => return Ok(HttpResponse::NotFound().body("Patient not found")),
  };

  state.sessions.retain(|session| session.patient_uuid != uuid);

  patient.delete();
  state.broadcast(SseEvent::PatientRemoved(&uuid)).await;
  
  info!("Deleted patient {}", patient.name);
  Ok(HttpResponse::Ok().body("Patient deleted"))
}