use super::patient::Patient;
use super::session::Session;
use crate::consts;

use std::{fs, io};

const SECRETS: &'static str = include_str!("../../secrets.json");

#[derive(Debug, Clone, Default)]
pub struct State {
  pub session: Vec<Session>,
  pub patient: Vec<Patient>,
  pub secrets: Secrets,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
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
      secrets,
    })
  }
}