use std::path::Path;
use std::{fs, io};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {}

impl Session {
  pub fn new() -> Self {
    todo!()
  }

  pub fn from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
    let file = fs::read_to_string(path)?;
    let session = serde_json::from_str(&file)?;
    Ok(session)
  }

  pub fn from_dir<P: AsRef<Path>>(path: P) -> io::Result<Vec<Self>> {
    let dir = fs::read_dir(path)?;
    let mut sessions = Vec::new();

    for entry in dir {
      let path = entry?.path();
      if path.is_file() {
        let session = Session::from_file(path)?;
        sessions.push(session);
      }
    }

    Ok(sessions)
  }
}