use std::process::exit;

use crate::state::state::{UserInfo, User};
use crate::AppState;
use crate::consts;
use crate::logs::*;

use actix_web::{Responder, web, Either};
use chrono::Utc;
use reqwest::Client;
use tokio::sync::mpsc;

#[actix_web::get("/authorize")]
pub async fn index(state: web::Data<AppState>) -> impl Responder {
  let appstate = state.read().await;
  let url = format!("https://accounts.google.com/o/oauth2/v2/auth?scope={}&access_type=offline&response_type=code&redirect_uri={}&client_id={}&prompt=consent",
    consts::SCOPES.join(" "),
    format!("{}/oauth", consts::URL),
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

#[derive(serde::Deserialize)]
pub struct GoogleResp {
  access_token: String,
  expires_in: u64,
  refresh_token: String,
}

// Todo: respond with index.html
#[actix_web::get("/oauth")]
pub async fn oauth(state: web::Data<AppState>, query: web::Query<Query>) -> Either<String, web::Redirect> {
  let (code, scopes) = match query.into_inner() {
    Query::Success { code, scope } => (code, scope),
    Query::Error { error } => return Either::Left(format!("Error: {}", error)),
  };
  
  let scopes = scopes.split(' ').collect::<Vec<&str>>();
  if consts::SCOPES.iter().any(|s| !scopes.contains(s)) {
    warning!("Someone tried to authorize with not enough scopes");
    return Either::Left("Error: invalid scopes".into());
  }

  let appstate = state.read().await;

  let client = Client::new();
  let res = client
  .post("https://oauth2.googleapis.com/token")
  .header("Content-Type", "application/x-www-form-urlencoded")
  .form(&[
    ("code", &code),
    ("client_id", &appstate.secrets.client_id),
    ("client_secret", &appstate.secrets.client_secret),
    ("redirect_uri", &format!("{}/oauth", consts::URL)),
    ("grant_type", &"authorization_code".into()),
  ])
  .send()
  .await;

  let res = match res {
    Ok(res) => res,
    Err(err) => return Either::Left(format!("Error: {}", err)),
  };

  let res = match res.json::<GoogleResp>().await {
    Ok(res) => res,
    Err(err) => return Either::Left(format!("Error: {}", err)),
  };

  let user = client.get("https://www.googleapis.com/oauth2/v2/userinfo")
    .bearer_auth(&res.access_token)
    .send()
    .await;

  let user = match user {
    Ok(user) => user,
    Err(err) => return Either::Left(format!("Error: {}", err)),
  };

  let user = match user.json::<UserInfo>().await {
    Ok(user) => user,
    Err(err) => return Either::Left(format!("Error: {}", err)),
  };

  let (tx, rx) = mpsc::channel(1);
  let user = User {
    access_token: res.access_token,
    user_info: user,
    expires_at: res.expires_in + Utc::now().timestamp() as u64,
    refresh_token: res.refresh_token,
    stop_tx: tx,
  };

  drop(appstate);
  let mut appstate = state.write().await;
  appstate.add_new_user(user, rx);

  Either::Right(web::Redirect::to(format!("{}/dashboard", consts::URL)))
}