use crate::AppState;
use crate::consts;

use actix_web::{Responder, web};

#[actix_web::get("/authorize")]
pub async fn index(state: web::Data<AppState>) -> impl Responder {
  let appstate = state.read().await;
  let url = format!("https://accounts.google.com/o/oauth2/v2/auth?scope={}&access_type=offline&response_type=code&redirect_uri={}&client_id={}",
    consts::SCOPE,
    consts::REDIRECT,
    appstate.secrets.client_id
  );

  web::Redirect::to(url).permanent()
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Query {
  Success { code: String, scope: String },
  Error { error: String },
}

#[actix_web::get("/oauth")]
pub async fn oauth(state: web::Data<AppState>, query: web::Query<Query>) -> impl Responder {
  ""
}