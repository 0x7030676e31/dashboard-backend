use super::patient::Patient;
use super::session::Session;
use crate::AppState;
use crate::logs::*;
use crate::consts;

use std::{fs, io};
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::{mpsc, RwLock};
use tokio::time;
use serde::{Serialize, Deserialize};

const SECRETS: &str = include_str!("../../secrets.json");

#[derive(Debug)]
pub struct State {
  pub session: Vec<Session>,
  pub patient: Vec<Patient>,
  pub users: Vec<Arc<RwLock<User>>>,
  pub secrets: Arc<Secrets>,
  pub write_tx: mpsc::Sender<()>,
}

#[derive(Debug, Clone)]
pub struct User {
  pub access_token: String,
  pub expires_at: u64,
  pub refresh_token: String,
  pub stop_tx: mpsc::Sender<()>,
  pub write_tx: mpsc::Sender<()>,
  pub secrets: Arc<Secrets>,
  pub user_info: UserInfo,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserInfo {
  pub id: String,
  pub email: Option<String>,
  pub verified_email: Option<bool>,
  pub name: String,
  pub given_name: String,
  pub picture: String,
  pub locale: String,
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

impl State {
  pub fn new() -> io::Result<(Self, mpsc::Receiver<()>)> {
    let sessions_dir = format!("{}/sessions", consts::PATH);
    let patients_dir = format!("{}/patients", consts::PATH);

    if fs::metadata(&sessions_dir).is_err() {
      fs::create_dir_all(&sessions_dir)?;
    }

    if fs::metadata(&patients_dir).is_err() {
      fs::create_dir_all(&patients_dir)?;
    }

    let secrets = serde_json::from_str(SECRETS)?;
    let (write_tx, write_rx) = mpsc::channel(1);

    Ok((State {
      session: Session::from_dir(&sessions_dir)?,
      patient: Patient::from_dir(&patients_dir)?,
      users: Vec::new(),
      secrets: Arc::new(secrets),
      write_tx,
    }, write_rx))
  }

  pub fn start_write_loop(state: AppState, mut write_rx: mpsc::Receiver<()>) {
    tokio::spawn(async move {
      info!("Starting write loop...");
      loop {
        write_rx.recv().await.unwrap();
        info!("Writing state to disk...");
      }
    });
  }

  pub fn add_new_user(&mut self, user: User, stop_rx: mpsc::Receiver<()>) {
    let user = Arc::new(RwLock::new(user));
    self.users.push(user.clone());
    
    tokio::spawn(async move {
      Self::start_refresh_loop(user, stop_rx).await;
    });
  }

  pub async fn start_refresh_loop(user: Arc<RwLock<User>>, mut stop_rx: mpsc::Receiver<()>) {
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

}