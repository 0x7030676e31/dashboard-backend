use super::state::Secrets;

use std::sync::Arc;

use serde::{Serialize, Deserialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct User {
  pub access_token: String,
  pub expires_at: u64,
  pub refresh_token: String,
  pub user_info: UserInfo,
  pub settings: Settings,

  pub stop_tx: mpsc::Sender<()>,
  pub write_tx: mpsc::Sender<()>,
  pub secrets: Arc<Secrets>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Settings {
  pub google_calendar_enabled: bool,
}

impl Default for Settings {
  fn default() -> Self {
    Settings {
      google_calendar_enabled: false,
    }
  }
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RwUser {
  pub id: String,
  pub email: String,
  pub verified_email: bool,
  pub name: String,
  pub given_name: String,
  pub picture: String,
  pub locale: String,

  pub access_token: String,
  pub expires_at: u64,
  pub refresh_token: String,
}

impl RwUser {
  pub fn from_user(u: &User) -> Self {
    RwUser {
      id: u.user_info.id.clone(),
      email: u.user_info.email.clone(),
      verified_email: u.user_info.verified_email,
      name: u.user_info.name.clone(),
      given_name: u.user_info.given_name.clone(),
      picture: u.user_info.picture.clone(),
      locale: u.user_info.locale.clone(),

      access_token: u.access_token.clone(),
      expires_at: u.expires_at,
      refresh_token: u.refresh_token.clone(),
    }
  }
}