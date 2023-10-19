use super::patient::Patient;
use super::session::Session;
use crate::consts;

use std::{fs, io};

use tokio::sync::mpsc;
use serde::{Serialize, Deserialize};

const SECRETS: &'static str = include_str!("../../secrets.json");

#[derive(Debug, Clone)]
pub struct State {
  pub session: Vec<Session>,
  pub patient: Vec<Patient>,
  pub users: Vec<User>,
  pub secrets: Secrets,
}

#[derive(Debug, Clone)]
pub struct User {
  pub access_token: String,
  pub expires_at: u64,
  pub refresh_token: String,
  pub stop_tx: mpsc::Sender<()>,
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

impl State {
  pub fn new() -> io::Result<Self> {
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
      users: Vec::new(),
      secrets,
    })
  }

  pub fn add_new_user(&mut self, user: User, _stop_rx: mpsc::Receiver<()>) {
    println!("Adding new user: {:?}", user);
  } 
}