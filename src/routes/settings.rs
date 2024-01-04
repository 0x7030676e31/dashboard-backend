use crate::AppState;
use crate::logs::*;

use actix_web::post;
use actix_web::{patch, HttpResponse, HttpRequest, web, Error};
use serde::Deserialize;

#[derive(Deserialize)]
struct PartialSettings {
  google_calendar_enabled: Option<bool>,
}

#[patch("/settings")]
pub async fn index(req: HttpRequest, state: web::Data<AppState>, body: web::Json<PartialSettings>) -> Result<HttpResponse, Error> {
  let app_state = state.read().await;
  let user = app_state.auth_token(req)?;
  let mut user = match app_state.users.get(&user) {
    Some(user) => user.write().await,
    None => {
      error!("Couldn't find user for token {}", user);
      return Ok(HttpResponse::InternalServerError().body("Internal Server Error"));
    }
  };

  if let Some(google_calendar_enabled) = body.google_calendar_enabled {
    user.settings.google_calendar_enabled = google_calendar_enabled;
  }

  info!("Updated settings for user {}", user.user_info.email);

  app_state.write();
  Ok(HttpResponse::NoContent().finish())
}

#[post("/settings/resync")]
pub async fn google_calendar_resync(req: HttpRequest, state: web::Data<AppState>) -> Result<HttpResponse, Error> {
  let app_state = state.read().await;
  let user = app_state.auth_token(req)?;
  let mut _user = match app_state.users.get(&user) {
    Some(user) => user.write().await,
    None => {
      error!("Couldn't find user for token {}", user);
      return Ok(HttpResponse::InternalServerError().body("Internal Server Error"));
    }
  };

  Ok(HttpResponse::NoContent().finish())
}