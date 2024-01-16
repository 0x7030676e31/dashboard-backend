use std::collections::hash_map::Entry;
use std::sync::Arc;

use crate::state::state::ArcState;
use crate::state::user::{UserInfo, User};
use crate::{AppState, EnvVars};
use crate::consts;
use crate::logs::*;

use actix_web::{Responder, web, Either, HttpResponse, HttpRequest};
use chrono::Utc;
use futures::future;
use reqwest::ClientBuilder;
use tokio::sync::mpsc;

#[actix_web::get("/authorize")]
pub async fn index(req: HttpRequest, state: web::Data<AppState>, env: web::Data<EnvVars>) -> impl Responder {
  let origin = if env.is_production {
    let origin = req.connection_info().host().to_owned();
    format!("https://{}", if let Some(strip) = origin.strip_prefix("www.") { strip } else { &origin })
  } else {
    format!("http://localhost:{}", env.inner_port)
  };
  
  let appstate = state.read().await;
  let url = format!("https://accounts.google.com/o/oauth2/v2/auth?scope={}&access_type=offline&response_type=code&redirect_uri={}/oauth&client_id={}&prompt=consent",
    consts::SCOPES.join(" "),
    origin,
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

#[actix_web::get("/oauth")]
pub async fn oauth(req: HttpRequest, state: web::Data<AppState>, env: web::Data<EnvVars>, query: web::Query<Query>) -> Either<String, web::Redirect> {
  let origin = if env.is_production {
    let origin = req.connection_info().host().to_owned();
    format!("https://{}", if let Some(strip) = origin.strip_prefix("www.") { strip } else { &origin })
  } else {
    format!("http://localhost:{}", env.inner_port)
  };

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

  let client = ClientBuilder::new()
    .danger_accept_invalid_certs(true)
    .build()
    .unwrap();

  let res = client
    .post("https://oauth2.googleapis.com/token")
    .header("Content-Type", "application/x-www-form-urlencoded")
    .form(&[
      ("code", &code),
      ("client_id", &appstate.secrets.client_id),
      ("client_secret", &appstate.secrets.client_secret),
      ("redirect_uri", &format!("{}/oauth", origin)),
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

  if env.users.iter().all(|mail| mail != &user.email) {
    warning!("Someone tried to authorize with an unauthorized email: {}", user.email);
    return Either::Left("Error: unauthorized email".into());
  }

  drop(appstate);
  
  let mut appstate = state.write().await;
  let futures = appstate.users.values().map(|u| u.read());
  let token = future::join_all(futures).await.iter().enumerate().find_map(|(i, u)| {
    if u.user_info.email == user.email { Some(appstate.users.keys().nth(i).unwrap().to_owned()) }
    else { None }
  });

  let token = match token {
    Some(token) => token,
    None => {
      let (tx, rx) = mpsc::channel(1);
      let write_tx = appstate.write_tx.clone();

      let user = User {
        access_token: res.access_token,
        user_info: user,
        settings: Default::default(),
        expires_at: res.expires_in + Utc::now().timestamp() as u64,
        refresh_token: res.refresh_token,
        stop_tx: tx,
        write_tx,
        secrets: Arc::clone(&appstate.secrets),
      };

      
      appstate.add_new_user(user, rx)
    }
  };
  
  drop(appstate);
  let code = state.new_code(&token).await;

  let origin = if env.is_production { origin } else { format!("http://localhost:{}", env.dev_port) };
  Either::Right(web::Redirect::to(format!("{}/dashboard?code={}", origin, code)))
}

#[actix_web::post("/auth")]
pub async fn auth(state: web::Data<AppState>, code: web::Bytes) -> impl Responder {
  let mut appstate = state.write().await;
  let code = String::from_utf8(code.to_vec()).unwrap();
  match appstate.auth_codes.entry(code) {
    Entry::Occupied(entry) => {
      let token = entry.get().clone();
      entry.remove();
      
      info!("User {} authenticated", token);
      HttpResponse::Ok().body(token)
    },
    Entry::Vacant(_) => {
      error!("Someone tried to authenticate with an invalid code");
      HttpResponse::BadRequest().body("Invalid code")
    },
  }
}

