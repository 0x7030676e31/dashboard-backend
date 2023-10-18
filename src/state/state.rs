use std::{fs, io};

use super::patient::Patient;
use super::session::Session;

const SECRETS: &'static str = include_str!("../../secrets.json");

#[derive(Debug, Clone, Default)]
pub struct State {
  pub session: Vec<Session>,
  pub patient: Vec<Patient>,
}

#[cfg(not(production))]
pub const PATH: &str = concat!(env!("HOME"), "/.config/dashboard");

#[cfg(production)]
pub const PATH: &str = "./";

impl State {
  pub fn new() -> io::Result<Self> {
    let sessions_dir = format!("{}/sessions", PATH);
    let patients_dir = format!("{}/patients", PATH);

    if fs::metadata(&sessions_dir).is_err() {
      fs::create_dir_all(&sessions_dir)?;
    }

    if fs::metadata(&patients_dir).is_err() {
      fs::create_dir_all(&patients_dir)?;
    }

    Ok(State {
      session: Session::from_dir(&sessions_dir)?,
      patient: Patient::from_dir(&patients_dir)?,
    })
  }
}