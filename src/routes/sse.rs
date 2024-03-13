use crate::AppState;
use crate::logs::*;
use crate::state::state::SseEvent;
use crate::state::state::State;

use actix_web::Either;
use actix_web::Responder;
use actix_web::{web, HttpResponse, HttpRequest};
use actix_web_lab::sse;
use actix_web_lab::sse::Sse;
use serde::Deserialize;
use tokio::sync::mpsc;

#[derive(Deserialize)]
struct SseInfo {
  token: String,
}

#[actix_web::get("/sse/token")]
pub async fn get_token(req: HttpRequest, state: web::Data<AppState>) -> Result<HttpResponse, actix_web::Error> {
  let app_state = state.read().await;
  let token = app_state.auth_token(req)?;
  
  let sse_token = match app_state.sse_tokens.get(&token) {
    Some(sse_token) => sse_token,
    None => {
      error!("Couldn't find sse token for user {}", token);
      return Ok(HttpResponse::Unauthorized().body("Unauthorized"));
    }
  };

  Ok(HttpResponse::Ok().body(sse_token.to_owned()))
}

#[actix_web::get("/sse/stream")]
pub async fn stream(state: web::Data<AppState>, query: web::Query<SseInfo>) -> Either<HttpResponse, impl Responder> {
  let mut app_state = state.write().await;
  let token = match app_state.sse_tokens.iter().find_map(|(k, v)| if v == &query.token { Some(k) } else { None }) {
    Some(token) => token.to_owned(),
    None => {
      error!("Couldn't find sse token for user {}", query.token);
      return Either::Left(HttpResponse::Unauthorized().body("Unauthorized"));
    }
  };

  let user = match app_state.users.get(&token) {
    Some(user) => user.read().await,
    None => {
      error!("Couldn't find user for token {}", token);
      return Either::Left(HttpResponse::InternalServerError().body("Internal Server Error"));
    }
  };

  let (tx, rx) = mpsc::channel(10);

  let msg = SseEvent::Ready {
    patients: &app_state.patients,
    sessions: &app_state.sessions,
    user_mail: &user.user_info.email,
    user_avatar: &user.user_info.picture,
    settings: &user.settings,
  };

  let tx2 = tx.clone();
  let msg = serde_json::to_string(&msg).unwrap();
  drop(user);
  
  info!("User {} connected to sse", token.clone());
  tokio::spawn(async move {
    if let Err(err) = tx2.send(sse::Data::new(msg).into()).await {
      error!("Couldn't send message to sse channel: {}", err);
    }
  });
  
  app_state.sse.push((token.clone(), tx));
  app_state.sse_tokens.entry(token).and_modify(|e| *e = State::generate_token());
  
  drop(app_state);
  Either::Right(Sse::from_infallible_receiver(rx))
}