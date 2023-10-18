use crate::consts;

use std::path::Path;
use std::{fs, io};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Patient {
  pub uuid: String,
}

impl Patient {
  pub fn new() -> Self {
    todo!()
  }

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

    Ok(patients)
  }

  pub fn write(&self) {
    let _path = format!("{}/{}.json", consts::PATH, self.uuid);
    // match fs::write(path, serde_json::to_string(self).unwrap()) {
  }
}