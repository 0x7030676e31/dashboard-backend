use super::user::User;
use super::patient::Patient;
use super::session::Session;
use crate::AppState;
use crate::logs::*;
use crate::consts;

use std::collections::HashMap;
use std::{fs, io};
use std::sync::Arc;

use chrono::Utc;
use sha2::{Sha256, Digest};
use tokio::sync::{mpsc, RwLock};
use tokio::time;
use serde::Deserialize;

const SECRETS: &str = include_str!("../../secrets.json");

#[derive(Debug)]
pub struct State {
  pub session: Vec<Session>,
  pub patient: Vec<Patient>,
  pub users: HashMap<String, Arc<RwLock<User>>>,
  pub secrets: Arc<Secrets>,
  pub write_tx: mpsc::Sender<()>,
  pub auth_codes: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Secrets {
  pub client_id: String,
  pub client_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct GoogleRefreshResp {
  pub access_token: String,
  pub expires_in: u64,
}

#[async_trait::async_trait]
pub trait ArcState {
  fn fs_write(&self);
  async fn new_code(&self, token: &str) -> String;
}

#[async_trait::async_trait]
impl ArcState for AppState {
  fn fs_write(&self) {
    let state = self.clone();
    tokio::spawn(async move {
      let state = state.read().await;
      state.write();
    });
  }

  async fn new_code(&self, token: &str) -> String {
    let mut state = self.write().await;
    let code = State::generate_token();
    state.auth_codes.insert(code.clone(), token.to_string());
    info!("Created new auth code: {}", code);

    let state = self.clone();
    let code2 = code.clone();
    tokio::spawn(async move {
      time::sleep(time::Duration::from_secs(60)).await;
      let mut state = state.write().await;
      state.auth_codes.remove(&code2);
      info!("Removed auth code: {}", code2);
    });
    
    code
  }
}

impl State {
  pub fn generate_token() -> String {
    let mut hasher = Sha256::new();
    hasher.update((i64::MAX - Utc::now().timestamp()).to_string());
    format!("{:x}", hasher.finalize())
  }

  pub fn new(write_tx: mpsc::Sender<()>) -> io::Result<Self> {
    let sessions_dir = format!("{}/sessions", consts::PATH);
    let patients_dir = format!("{}/patients", consts::PATH);

    if fs::metadata(&sessions_dir).is_err() {
      fs::create_dir_all(&sessions_dir)?;
    }

    if fs::metadata(&patients_dir).is_err() {
      fs::create_dir_all(&patients_dir)?;
    }

    let secrets = serde_json::from_str(SECRETS)?;

    Ok(State {
      session: Session::from_dir(&sessions_dir)?,
      patient: Patient::from_dir(&patients_dir)?,
      users: HashMap::new(),
      secrets: Arc::new(secrets),
      write_tx,
      auth_codes: HashMap::new(),
    })
  }

  pub fn start_write_loop(state: AppState, mut write_rx: mpsc::Receiver<()>) {
    tokio::spawn(async move {
      info!("Starting write loop...");
      loop {
        if write_rx.recv().await.is_none() {
          info!("Closing write loop...");
          break;
        }

        info!("Writing state to disk...");
        let state = state.read().await;
        state.write();
      }
    });
  }

  pub fn write(&self) {
    // dbg!("Writing state to disk...");
  }


  pub fn add_new_user(&mut self, user: User, stop_rx: mpsc::Receiver<()>) -> String {
    let user = Arc::new(RwLock::new(user));
    let token = Self::generate_token();
    self.users.insert(token.clone(), Arc::clone(&user));
    self.write();
    
    tokio::spawn(async move {
      Self::start_refresh_loop(user, stop_rx).await;
    });

    token
  }

  pub async fn start_refresh_loop(user: crate::User, mut stop_rx: mpsc::Receiver<()>) {
    info!("Starting refresh loop for user {}", user.read().await.user_info.name);
    loop {
      let rw_user = user.read().await;
      let refresh_in = rw_user.expires_at.saturating_sub(Utc::now().timestamp() as u64).saturating_sub(60);
      drop(rw_user);

      tokio::select! {
        _ = time::sleep(time::Duration::from_secs(refresh_in)) => {},
        _ = stop_rx.recv() => {
          info!("Stopping refresh loop for user {}", user.read().await.user_info.name);
          break;
        }
      }

      let mut rw_user = user.write().await;
      let client = reqwest::Client::new();
      let res = client
        .post("https://oauth2.googleapis.com/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
          ("client_id", &rw_user.secrets.client_id),
          ("client_secret", &rw_user.secrets.client_secret),
          ("refresh_token", &rw_user.refresh_token),
          ("grant_type", &"refresh_token".into()),
        ])
        .send()
        .await;
    
      let res = match res {
        Ok(res) => res,
        Err(err) => {
          error!("There was an error while refreshing the token for user {}: {}", rw_user.user_info.name, err);
          continue;
        },
      };

      let res = match res.json::<GoogleRefreshResp>().await {
        Ok(res) => res,
        Err(err) => {
          error!("There was an error while refreshing the token for user {}: {}", rw_user.user_info.name, err);
          continue;
        },
      };

      rw_user.access_token = res.access_token;
      rw_user.expires_at = res.expires_in + Utc::now().timestamp() as u64;
      
      let tx = rw_user.write_tx.clone();
      tokio::spawn(async move {
        tx.send(()).await.unwrap();
      });

      drop(rw_user);
    }
  }

  pub async fn get_user(&self, req: &actix_web::HttpRequest) -> Result<crate::User, actix_web::Error> {
    let auth = match req.headers().get("Authorization") {
      Some(auth) => auth.to_str().unwrap_or("").to_string(),
      None => return Err(actix_web::error::ErrorUnauthorized("Missing Authorization header")),
    };

    match self.users.get(&auth) {
      Some(user) => Ok(user.clone()),
      None => Err(actix_web::error::ErrorUnauthorized("Invalid Authorization header")),
    }
  }
}