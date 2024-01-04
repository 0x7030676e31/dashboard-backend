use crate::logs::*;

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

#[derive(Serialize, Deserialize)]
struct FsPatient {
  description: String,
  age: Option<u8>,
  name: String,
  address: String,
  profile_picture: Option<String>,
  created_at: u64,
  last_updated: u64,
}

impl Patient {
  pub fn from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
    let file = fs::read_to_string(path.as_ref())?;
    let fs_patient: FsPatient = serde_json::from_str(&file)?;
    let patient = Patient {
      uuid: path.as_ref().file_stem().unwrap().to_str().unwrap().to_owned(),
      description: fs_patient.description,
      age: fs_patient.age,
      name: fs_patient.name,
      address: fs_patient.address,
      profile_picture: fs_patient.profile_picture,
      created_at: fs_patient.created_at,
      last_updated: fs_patient.last_updated,
    };

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
    let fs_patient = FsPatient {
      description: self.description.clone(),
      age: self.age,
      name: self.name.clone(),
      address: self.address.clone(),
      profile_picture: self.profile_picture.clone(),
      created_at: self.created_at,
      last_updated: self.last_updated,
    };

    if let Err(err) = fs::write(path, serde_json::to_string(&fs_patient).unwrap()) {
      error!("Couldn't write patient file: {}", err);
    }
  }

  pub fn delete(&self) {
    let path = format!("{}patients/{}.json", fspath!(), self.uuid);
    if let Err(err) = fs::remove_file(path) {
      error!("Couldn't delete patient file: {}", err);
    }
  }
}