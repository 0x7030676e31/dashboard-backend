use crate::logs::*;

use std::env;
use std::path::Path;
use std::{fs, io};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Patient {
  pub uuid: String,
  pub description: String,
  pub age: Option<u8>,
  pub name: String,
  pub address: String,
  pub profile_picture: Option<String>,
  pub created_at: u64,
  pub last_updated: u64,
}

impl Patient {
  pub fn from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
    let file = fs::read_to_string(path)?;
    let patient = serde_json::from_str(&file)?;
    Ok(patient)
  }

  pub fn from_dir<P: AsRef<Path>>(path: P) -> io::Result<Vec<Self>> {
    let dir = fs::read_dir(path)?;
    let mut patients = Vec::new();

    for entry in dir {
      let path = entry?.path();
      if path.is_file() {
        let patient = Patient::from_file(path)?;
        patients.push(patient);
      }
    }

    info!("Loaded {} patients", patients.len());
    Ok(patients)
  }

  pub fn write(&self) {
    let path = format!("{}patients/{}.json", fspath!(), self.uuid);
    if let Err(err) = fs::write(path, serde_json::to_string(self).unwrap()) {
      error!("Couldn't write patient to file: {}", err);
    }
  }

  pub fn delete(&self) {
    let path = format!("{}patients/{}.json", fspath!(), self.uuid);
    if let Err(err) = fs::remove_file(path) {
      error!("Couldn't delete patient file: {}", err);
    }
  }
}