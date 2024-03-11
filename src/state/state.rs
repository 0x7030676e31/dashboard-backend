use super::user::{User, RwUser, Settings};
use super::patient::Patient;
use super::session::Session;
use crate::AppState;
use crate::logs::*;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::{fs, io};
use std::sync::Arc;

use actix_web::HttpRequest;
use actix_web_lab::sse;
use chrono::Utc;
use reqwest::ClientBuilder;
use sha2::{Sha256, Digest};
use tokio::sync::{mpsc, RwLock};
use tokio::time;
use serde::{Serialize, Deserialize};
use tokio::time::interval;
use futures::future;
use uuid::Uuid;

const SECRETS: &str = include_str!("../../secrets.json");

#[derive(Debug)]
pub struct State {
  pub sessions: Vec<Session>,
  pub patients: Vec<Patient>,
  pub users: HashMap<String, Arc<RwLock<User>>>,
  pub path: String,
  pub calendar_webhooks: HashMap<String, GoogleWebhook>,

  // <Access token, SSE token>
  pub sse_tokens: HashMap<String, String>,
  pub sse: Vec<mpsc::Sender<sse::Event>>,
  pub secrets: Arc<Secrets>,
  pub write_tx: mpsc::Sender<()>,

  // <Verification code, Access token>
  pub auth_codes: HashMap<String, String>,

  // Used to identify messages
  pub ack: AtomicU64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleWebhook {
  pub uuid: String,
  pub resource_id: String,
  pub expiry: u64,
  pub events: Vec<String>,
  pub sync_token: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleWebhookResp {
  resource_id: String,
  expiration: String,
} 

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EventsListResp {
  pub next_sync_token: String,
  pub items: Vec<GoogleEvent>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GoogleEvent {
  id: String,
  status: Option<String>,
  color_id: Option<String>,
  summary: Option<String>,
  start: DateTime,
  end: DateTime,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DateTime {
  date_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RwState {
  users: HashMap<String, RwUser>,
  sse_tokens: HashMap<String, String>,
  calendar_webhooks: Option<HashMap<String, GoogleWebhook>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Secrets {
  pub client_id: String,
  pub client_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct GoogleRefreshResp {
  pub access_token: String,
  pub expires_in: u64,
}

#[allow(dead_code)]
#[async_trait::async_trait]
pub trait ArcState {
  fn fs_write(&self);
  async fn new_code(&self, token: &str) -> String;
}

#[async_trait::async_trait]
impl ArcState for AppState {
  fn fs_write(&self) {
    let state = self.clone();
    tokio::spawn(async move {
      let state = state.read().await;
      state.write();
    });
  }

  async fn new_code(&self, token: &str) -> String {
    let mut state = self.write().await;
    let code = State::generate_token();
    state.auth_codes.insert(code.clone(), token.to_string());
    info!("Created new auth code: {}", code);

    let state = self.clone();
    let code2 = code.clone();
    tokio::spawn(async move {
      time::sleep(time::Duration::from_secs(60)).await;
      let mut state = state.write().await;
      if state.auth_codes.remove(&code2).is_some() {
        info!("Removed auth code: {}", code2);
      }
    });
    
    code
  }
}

impl State {
  pub fn generate_token() -> String {
    let mut hasher = Sha256::new();
    hasher.update((i64::MAX - Utc::now().timestamp()).to_string());
    format!("{:x}", hasher.finalize())
  }

  pub fn new(write_tx: mpsc::Sender<()>, file_path: String) -> io::Result<Self> {
    let sessions_dir = format!("{}sessions", file_path);
    let patients_dir = format!("{}patients", file_path);
    let path = format!("{}state.json", file_path);

    if fs::metadata(&sessions_dir).is_err() {
      fs::create_dir_all(&sessions_dir)?;
    }

    if fs::metadata(&patients_dir).is_err() {
      fs::create_dir_all(&patients_dir)?;
    }

    let secrets = serde_json::from_str(SECRETS)?;
    if fs::metadata(&path).is_err() {
      info!("No state file found, creating empty state...");
      return Ok(State {
        sessions: Session::from_dir(&sessions_dir)?,
        patients: Patient::from_dir(&patients_dir)?,
        users: HashMap::new(),
        path: file_path,
        calendar_webhooks: HashMap::new(),
        sse_tokens: HashMap::new(),
        sse: Vec::new(),
        secrets: Arc::new(secrets),
        write_tx,
        auth_codes: HashMap::new(),
        ack: AtomicU64::new(0),
      });
    }

    let file = fs::read_to_string(&path)?;
    let rwstate = serde_json::from_str::<RwState>(&file)?;

    let secrets = Arc::new(secrets);
    let mut users = HashMap::new();

    for (token, user) in rwstate.users {
      let (tx, rx) = mpsc::channel(1);
      let write_tx = write_tx.clone();
      
      let user = User {
        access_token: user.access_token,
        expires_at: user.expires_at,
        refresh_token: user.refresh_token,
        settings: user.settings,
        user_info: crate::state::user::UserInfo {
          id: user.id,
          email: user.email,
          verified_email: user.verified_email,
          name: user.name,
          given_name: user.given_name,
          picture: user.picture,
          locale: user.locale,
        },
        stop_tx: tx,
        write_tx,
        secrets: Arc::clone(&secrets),
      };

      let user = Arc::new(RwLock::new(user));
      users.insert(token, Arc::clone(&user));

      tokio::spawn(async move {
        Self::start_refresh_loop(user, rx).await;
      });
    }
    
    info!("Loaded state from disk, found {} users", users.len());
    Ok(State {
      sessions: Session::from_dir(&sessions_dir)?,
      patients: Patient::from_dir(&patients_dir)?,
      users,
      path: file_path,
      calendar_webhooks: rwstate.calendar_webhooks.unwrap_or_default(),
      sse_tokens: rwstate.sse_tokens,
      sse: Vec::new(),
      secrets,
      write_tx,
      auth_codes: HashMap::new(),
      ack: AtomicU64::new(0),
    })
  }

  pub fn spawn_ping_loop(state: AppState) {
    tokio::spawn(async move {
      info!("Starting ping loop...");
      let mut interval = interval(Duration::from_secs(10));

      loop {
        interval.tick().await;
        state.write().await.remove_stale_clients().await;
      }
    });
  }

  async fn remove_stale_clients(&mut self) {
    let clients = self.sse.clone();
    
    let mut active_clients = Vec::new();
    for client in clients {
      let is_ok = client.send(sse::Event::Comment("ping".into())).await.is_ok();
      if is_ok {
        active_clients.push(client);
      }
    }

    self.sse = active_clients;
  }

  pub fn start_write_loop(state: AppState, mut write_rx: mpsc::Receiver<()>) {
    tokio::spawn(async move {
      info!("Starting write loop...");
      loop {
        if write_rx.recv().await.is_none() {
          info!("Closing write loop...");
          break;
        }

        let state = state.read().await;
        state.write();
      }
    });
  }

  pub fn write(&self) {
    let path = format!("{}state.json", self.path);
    let users = self.users.clone();
    let sse_tokens = self.sse_tokens.clone();
    let webhooks = self.calendar_webhooks.clone();

    tokio::spawn(async move {
      let bare_users = users.values().map(|u| u.read());
      let bare_users = future::join_all(bare_users).await;

      let rwstate = RwState {
        users: users.keys().enumerate().map(|(i, token)| (token.clone(), RwUser::from_user(&bare_users[i]))).collect(),
        sse_tokens,
        calendar_webhooks: Some(webhooks),
      };

      let json = serde_json::to_string(&rwstate).unwrap();
      if let Err(err) = fs::write(path, json) {
        error!("There was an error while writing the state to disk: {}", err);
      }
    });
  }

  pub async fn schedule_webhook_refresh(state: &AppState) {
    let app_state = state.read().await;

    for (user, webhook) in app_state.calendar_webhooks.iter() {
      let state = state.clone();
      let user = user.clone();
      
      let diff = webhook.expiry - chrono::Utc::now().timestamp_millis() as u64;
      tokio::spawn(async move {
        info!("Scheduling Google webhook refresh for user {} in {} min", user, diff / 1000 / 60);
        time::sleep(time::Duration::from_millis(diff)).await;
        Self::create_google_webhook(state, user);
      });
    }
  }

  pub async fn init_google_webhooks(state: &AppState) {
    let app_state = state.read().await;
    let users_to_add = app_state.users.keys().filter(|token| app_state.calendar_webhooks.get(*token).is_none()).map(|s| s.clone()).collect::<Vec<_>>();
    drop(app_state);

    for user in users_to_add {
      info!("Creating Google webhook for user {}", user);
      Self::create_google_webhook(Arc::clone(&state), user);
    }
  }

  pub fn create_google_webhook(state: AppState, user: String) {
    tokio::spawn(async move {    
      loop {
        let uuid = Uuid::new_v4().to_string();
        
        let app_state = state.read().await;
        let auth = format!("Bearer {}", app_state.users.get(&user).unwrap().read().await.access_token);
        drop(app_state);

        let client = ClientBuilder::new()
          .danger_accept_invalid_certs(true)
          .build()
          .unwrap();

        let res = client
          .post("https://www.googleapis.com/calendar/v3/calendars/primary/events/watch")
          .header("Authorization", &auth)
          .header("Content-Type", "application/json")
          .json(&serde_json::json!({
            "id": uuid,
            "type": "web_hook",
            "address": "https://entitia.com/api/events/webhook",
          }))
          .send()
          .await;

        let res = match res {
          Ok(res) => res,
          Err(err) => {
            error!("There was an error while creating the Google webhook for user {}: {}", user, err);
            return;
          },
        };

        let webhook = match res.json::<GoogleWebhookResp>().await {
          Ok(res) => res,
          Err(err) => {
            error!("There was an error while creating the Google webhook for user {}: {}", user, err);
            return;
          },
        };

        let then = chrono::Utc::now().checked_sub_months(chrono::Months::new(1)).unwrap();
        let then = then.to_rfc3339();

        info!("Created Google webhook for user {}", user);
        let res = client.get("https://www.googleapis.com/calendar/v3/calendars/primary/events")
          .query(&[
            ("timeMin", then.as_str()),
            ("maxResults", "2500"),
            ("singleEvents", "true"),
          ])
          .header("Authorization", auth)
          .send()
          .await;

        let res = match res {
          Ok(res) => res,
          Err(err) => {
            error!("There was an error while creating the Google webhook for user {}: {}", user, err);
            return;
          },
        };

        let res = match res.json::<EventsListResp>().await {
          Ok(res) => res,
          Err(err) => {
            error!("There was an error while creating the Google webhook for user {}: {}", user, err);
            return;
          },
        };

        let mut app_state = state.write().await;

        // If /events doesn't exists, create it
        if fs::metadata(format!("{}events", app_state.path)).is_err() {
          fs::create_dir(format!("{}events", app_state.path)).unwrap();
        }

        info!("Fetched {} events for user {}", res.items.len(), user);
        let events = serde_json::to_string(&res.items).unwrap();
        fs::write(format!("{}events/{}.json", app_state.path, user), events).unwrap();

        // Timestamp in ms
        let expiry = webhook.expiration.parse().unwrap();
        app_state.calendar_webhooks.insert(user.clone(), GoogleWebhook {
          uuid,
          resource_id: webhook.resource_id,
          expiry,
          events: res.items.iter().map(|e| e.id.clone()).collect(),
          sync_token: res.next_sync_token,
        });

        
        app_state.write();
        drop(app_state);
        
        info!("Created Google events file for user {}", user);
        let diff = expiry - chrono::Utc::now().timestamp_millis() as u64;
        time::sleep(time::Duration::from_millis(diff)).await;
      }
    });
  }

  pub fn auth_token(&self, req: HttpRequest) -> Result<String, actix_web::Error> {
    req.headers().get("Authorization").map_or(Err(actix_web::error::ErrorUnauthorized("Missing Authorization header")), |token| {
      let token = token.to_str().map_err(|_| actix_web::error::ErrorUnauthorized("Invalid Authorization header"))?;
      self.users.contains_key(token).then(|| token.to_string()).ok_or(actix_web::error::ErrorUnauthorized("Unauthorized"))
    })
  }

  pub fn check_auth(&self, req: HttpRequest) -> Result<Arc<RwLock<User>>, actix_web::Error> {
    req.headers().get("Authorization").map_or(Err(actix_web::error::ErrorUnauthorized("Missing Authorization header")), |token| {
      let token = token.to_str().map_err(|_| actix_web::error::ErrorUnauthorized("Invalid Authorization header"))?;
      self.users.get(token).map(Arc::clone).ok_or(actix_web::error::ErrorUnauthorized("Unauthorized"))
    })
  }

  pub fn add_new_user(&mut self, user: User, stop_rx: mpsc::Receiver<()>) -> String {
    let user = Arc::new(RwLock::new(user));
    let token = Self::generate_token();
    self.sse_tokens.insert(token.clone(), State::generate_token());
    self.users.insert(token.clone(), Arc::clone(&user));
    self.write();
    
    tokio::spawn(async move {
      Self::start_refresh_loop(user, stop_rx).await;
    });

    token
  }

  pub async fn start_refresh_loop(user: crate::User, mut stop_rx: mpsc::Receiver<()>) {
    info!("Starting refresh loop for user {}", user.read().await.user_info.name);
    loop {
      let rw_user = user.read().await;
      let refresh_in = rw_user.expires_at.saturating_sub(Utc::now().timestamp() as u64).saturating_sub(60);
      drop(rw_user);

      tokio::select! {
        _ = time::sleep(time::Duration::from_secs(refresh_in)) => {},
        _ = stop_rx.recv() => {
          info!("Stopping refresh loop for user {}", user.read().await.user_info.name);
          break;
        }
      }

      let mut rw_user = user.write().await;
      
      let client = ClientBuilder::new()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();
      let res = client
        .post("https://oauth2.googleapis.com/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
          ("client_id", &rw_user.secrets.client_id),
          ("client_secret", &rw_user.secrets.client_secret),
          ("refresh_token", &rw_user.refresh_token),
          ("grant_type", &"refresh_token".into()),
        ])
        .send()
        .await;
    
      let res = match res {
        Ok(res) => res,
        Err(err) => {
          error!("There was an error while refreshing the token for user {}: {}", rw_user.user_info.name, err);
          continue;
        },
      };

      let res = match res.json::<GoogleRefreshResp>().await {
        Ok(res) => res,
        Err(err) => {
          error!("There was an error while refreshing the token for user {}: {}", rw_user.user_info.name, err);
          continue;
        },
      };

      rw_user.access_token = res.access_token;
      rw_user.expires_at = res.expires_in + Utc::now().timestamp() as u64;
      // info!("Refreshed token for user {}", rw_user.user_info.name);

      let tx = rw_user.write_tx.clone();
      let username = rw_user.user_info.name.clone();
      tokio::spawn(async move {
        if let Err(err) = tx.send(()).await {
          error!("There was an error while sending the write signal for user {}: {}", username, err);
        }
      });

      drop(rw_user);
    }
  }

  async fn broadcast_message<'a>(&self, event: SseEvent<'a>, socket_ack: Option<u64>) {
    let ack = self.ack.fetch_add(1, Ordering::Relaxed);
    let msg = serde_json::to_string(&BroadcastMessage { payload: event, ack, socket_ack }).unwrap();
    info!("Broadcasting SSE message to {} clients", self.sse.len());

    let futs = self.sse.iter().map(|tx| tx.send(sse::Data::new(msg.clone()).into()));
    let res = future::join_all(futs).await;
    
    for (i, res) in res.into_iter().enumerate() {
      if let Err(err) = res {
        error!("Couldn't send message to sse client {}: {}", i, err);
      }
    }
  }

  pub async fn broadcast<'a>(&self, msg: SseEvent<'a>) {
    self.broadcast_message(msg, None).await;
  }

  pub async fn broadcast_socket<'a>(&self, msg: SseEvent<'a>, socket_ack: u64) {
    self.broadcast_message(msg, Some(socket_ack)).await;
  }
}

#[derive(Serialize)]
struct BroadcastMessage<'a> {
  #[serde(flatten)]
  payload: SseEvent<'a>,
  socket_ack: Option<u64>,
  ack: u64,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum SseEvent<'a> {
  Ready {
    patients: &'a Vec<Patient>,
    sessions: &'a Vec<Session>,
    user_mail: &'a str,
    user_avatar: &'a str,
    settings: &'a Settings,
  },
  PatientAdded(&'a Patient),
  PatientUpdated(&'a Patient),
  PatientRemoved(&'a String),
  SessionAdded(&'a Session),
  SessionUpdated(&'a Session),
  SessionRemoved(&'a String),
  // To be added
  EventAdded(&'a ()),
  EventUpdated(&'a ()),
  EventRemoved(&'a ()),
}

pub trait DrainWith<T> {
  fn drain_with<F>(&mut self, f: F) -> Vec<T> where F: FnMut(&mut T) -> bool;
}

impl<T> DrainWith<T> for Vec<T> {
  fn drain_with<F>(&mut self, mut f: F) -> Vec<T> where F: FnMut(&mut T) -> bool {
    let mut i = 0;
    let mut removed = Vec::new();
    while i < self.len() {
      if f(&mut self[i]) {
        removed.push(self.remove(i));
      } else {
        i += 1;
      }
    }

    removed
  }
}

