use super::state::Secrets;

use std::sync::Arc;

use tokio::sync::mpsc;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct User {
  pub access_token: String,
  pub expires_at: u64,
  pub refresh_token: String,
  pub user_info: UserInfo,

  pub stop_tx: mpsc::Sender<()>,
  pub write_tx: mpsc::Sender<()>,
  pub secrets: Arc<Secrets>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserInfo {
  pub id: String,
  pub email: String,
  pub verified_email: bool,
  pub name: String,
  pub given_name: String,
  pub picture: String,
  pub locale: String,
}

