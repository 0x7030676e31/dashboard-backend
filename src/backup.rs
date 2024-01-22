use crate::logs::*;

use std::fs;

use tokio::time::{Duration, interval};
use tokio::process::Command;
use chrono::prelude::*;

const BACKUP_INTERVAL: Duration = Duration::from_secs(60 * 30); // Backup every 30 minutes
const BACKUP_HISTORY: usize = 24 * 2 * 3; // Keep 3 days of backups

pub fn start_backup_loop(path: &String) {
  let backups_path = format!("{}backups", path);
  if let Err(err) = fs::create_dir_all(&backups_path) {
    error!("Failed to create backups directory: {}", err);
    return;
  }

  let path = path.clone();
  
  info!("Starting backup loop...");
  tokio::spawn(async move {
    let mut interval = interval(BACKUP_INTERVAL);
    
    loop {
      interval.tick().await;
      let output = Command::new("tar")
        .arg("-czf")
        .arg(format!("{}/{}.tar.gz", backups_path, Local::now().format("%Y-%m-%d_%H-%M-%S")))
        .arg("-C")
        .arg(&path)
        .arg("patients")
        .arg("sessions")
        .output()
        .await;
    
      match output {
        Ok(output) if !output.status.success() => {
          error!("Failed to create backup: {}", String::from_utf8_lossy(&output.stderr));
        },
        Err(err) => {
          error!("Failed to create backup: {}", err);
        },
        _ => {},
      };
      
      let mut backups = fs::read_dir(&backups_path).unwrap().map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
      backups.sort_by(|a, b| b.metadata().unwrap().modified().unwrap().cmp(&a.metadata().unwrap().modified().unwrap()));

      while backups.len() > BACKUP_HISTORY {
        let backup = backups.pop().unwrap();
        if let Err(err) = fs::remove_file(backup) {
          error!("Failed to remove backup: {}", err);
        }
      }
    }
  });
}
