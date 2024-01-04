use crate::AppState;
use crate::logs::*;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, Duration};
use std::collections::HashMap;
use std::path::Path;
use std::{fs, io};

use actix_web_actors::ws;
use actix::{Actor, StreamHandler, AsyncContext, ActorContext, Message, Handler, Addr};
use actix::Running;
use serde::{Deserialize, Serialize};
use tokio::time;

use super::state::SseEvent;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
  pub uuid: String,
  pub patient_uuid: String,
  pub start: u64,
  pub end: u64,
  pub paid: f32,
  pub emotions: Vec<Emotion>,
  pub timeline: HashMap<u64, TimelineEvent>,
  pub created_at: u64,
  pub last_updated: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct FsSession {
  patient_uuid: String,
  start: u64,
  end: u64,
  paid: f32,
  emotions: Vec<Emotion>,
  timeline: HashMap<u64, TimelineEvent>,
  created_at: u64,
  last_updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emotion {
  pub uuid: String,
  pub id: Option<u8>,
  pub kind: Option<u8>,
  pub aquired_age: Option<u8>,
  pub aquired_person: String,
  pub created_at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum TimelineEvent {
  SessionStart,
  SessionEnd,
  EmotionAdded(String),
  EmotionRemoved(String),
}

impl Session {
  pub fn from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
    let file = fs::read_to_string(path.as_ref())?;
    let fs_session: FsSession = serde_json::from_str(&file)?;
    let session = Session {
      uuid: path.as_ref().file_stem().unwrap().to_str().unwrap().to_string(),
      patient_uuid: fs_session.patient_uuid,
      start: fs_session.start,
      end: fs_session.end,
      paid: fs_session.paid,
      emotions: fs_session.emotions,
      timeline: fs_session.timeline,
      created_at: fs_session.created_at,
      last_updated: fs_session.last_updated,
    };
    
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

    info!("Loaded {} sessions", sessions.len());
    Ok(sessions)
  }

  pub fn write(&self) {
    let path = format!("{}sessions/{}.json", fspath!(), self.uuid);
    let fs_session = FsSession {
      patient_uuid: self.patient_uuid.clone(),
      start: self.start,
      end: self.end,
      paid: self.paid,
      emotions: self.emotions.clone(),
      timeline: self.timeline.clone(),
      created_at: self.created_at,
      last_updated: self.last_updated,
    };

    if let Err(err) = fs::write(path, serde_json::to_string(&fs_session).unwrap()) {
      error!("Couldn't write session to file: {}", err);
    }
  }

  pub fn delete(&self) {
    let path = format!("{}sessions/{}.json", fspath!(), self.uuid);
    if let Err(err) = fs::remove_file(path) {
      error!("Couldn't delete session file: {}", err);
    }
  }
}

#[derive(Deserialize, Debug)]
struct EditEmotion {
  uuid: String,
  id: Option<u8>,
  kind: Option<u8>,
  aquired_age: Option<u8>,
  aquired_person: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "payload")]
enum SocketMessage {
  AddEmotion(String),
  AddEmotionPrepend(String),
  EditEmotion(EditEmotion),
  RemoveEmotion(String),
  EditDescription(String),
}

#[derive(Debug, Clone)]
pub struct SessionSocket {
  state: AppState,
  uuid: String,
  authorized: Arc<AtomicBool>,
  addr: Option<Arc<Addr<SessionSocket>>>,
  hb: Instant,
  socket_id: u64,
  is_patient_update_scheduled: Arc<AtomicBool>,
  is_session_update_scheduled: Arc<AtomicBool>,
}

impl SessionSocket {
  pub fn new(state: AppState, uuid: String) -> Self {
    Self {
      state,
      uuid,
      authorized: Arc::new(AtomicBool::new(false)),
      addr: None,
      hb: Instant::now(),
      socket_id: chrono::Utc::now().timestamp_millis() as u64,
      is_patient_update_scheduled: Arc::new(AtomicBool::new(false)),
      is_session_update_scheduled: Arc::new(AtomicBool::new(false)),
    }
  }
  
  fn hb(&self, ctx: &mut ws::WebsocketContext<Self>) {
    ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
      if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
        info!("Disconnecting failed heartbeat");
        ctx.stop();
        return;
      }

      ctx.ping(b"hi");
    });
  }

  fn schedule_session_update(&self) {
    if self.is_session_update_scheduled.load(Ordering::Relaxed) {
      return;
    }

    self.is_session_update_scheduled.store(true, Ordering::Relaxed);
    let arc_state = Arc::new(self.clone());
    let state = self.state.clone();
    
    tokio::spawn(async move {
      time::sleep(Duration::from_secs(5)).await;
      let state = state.read().await;
      let session = match state.sessions.iter().find(|session| session.uuid == arc_state.uuid) {
        Some(session) => session,
        None => {
          error!("Couldn't find session to update");
          return;
        }
      };

      state.broadcast_socket(SseEvent::SessionUpdated(&session), arc_state.socket_id).await;
      arc_state.is_session_update_scheduled.store(false, Ordering::Relaxed);
    });
  }

  fn schedule_patient_update(&self) {
    if self.is_patient_update_scheduled.load(Ordering::Relaxed) {
      return;
    }

    self.is_patient_update_scheduled.store(true, Ordering::Relaxed);
    let arc_state = Arc::new(self.clone());
    let state = self.state.clone();
    
    tokio::spawn(async move {
      time::sleep(Duration::from_secs(5)).await;
      let state = state.read().await;
      let session = match state.sessions.iter().find(|session| session.uuid == arc_state.uuid) {
        Some(session) => session,
        None => {
          error!("Couldn't find session to update");
          return;
        }
      };

      let patient = match state.patients.iter().find(|patient| patient.uuid == session.patient_uuid) {
        Some(patient) => patient,
        None => {
          error!("Couldn't find patient to update");
          return;
        }
      };

      state.broadcast_socket(SseEvent::PatientUpdated(&patient), arc_state.socket_id).await;
      arc_state.is_patient_update_scheduled.store(false, Ordering::Relaxed);
    });
  }

  async fn handle_messge(&self, msg: String) -> Result<(), &'static str> {
    let msg: SocketMessage = serde_json::from_str(&msg).map_err(|_| "Couldn't parse message")?;
    let mut state = self.state.write().await;
    
    match msg {
      SocketMessage::AddEmotion(uuid) => {
        let session = state.sessions.iter_mut().find(|session| session.uuid == self.uuid).ok_or("Session not found")?;
        if session.emotions.iter().any(|emotion| emotion.uuid == uuid) {
          return Err("Emotion already exists");
        }

        let emotion = Emotion {
          uuid,
          id: None,
          kind: None,
          aquired_age: None,
          aquired_person: "".to_string(),
          created_at: chrono::Utc::now().timestamp() as u64,
        };

        session.emotions.push(emotion.clone());
        self.schedule_session_update();
        session.write();
      },
      SocketMessage::AddEmotionPrepend(uuid) => {
        let session = state.sessions.iter_mut().find(|session| session.uuid == self.uuid).ok_or("Session not found")?;
        if session.emotions.iter().any(|emotion| emotion.uuid == uuid) {
          return Err("Emotion already exists");
        }

        let emotion = Emotion {
          uuid,
          id: None,
          kind: None,
          aquired_age: None,
          aquired_person: "".to_string(),
          created_at: chrono::Utc::now().timestamp() as u64,
        };

        session.emotions.insert(0, emotion.clone());
        self.schedule_session_update();
        session.write();
      },
      SocketMessage::EditEmotion(edit) => {
        let session = state.sessions.iter_mut().find(|session| session.uuid == self.uuid).ok_or("Session not found")?;
        let emotion = session.emotions.iter_mut().find(|emotion| emotion.uuid == edit.uuid).ok_or("Emotion not found")?;

        if let Some(id) = edit.id {
          emotion.id = Some(id);
        }

        if let Some(kind) = edit.kind {
          emotion.kind = Some(kind);
        }

        if let Some(aquired_age) = edit.aquired_age {
          emotion.aquired_age = Some(aquired_age);
        }

        if !edit.aquired_person.is_empty() {
          emotion.aquired_person = edit.aquired_person;
        }

        self.schedule_session_update();
        session.write();
      },
      SocketMessage::RemoveEmotion(uuid) => {
        let session = state.sessions.iter_mut().find(|session| session.uuid == self.uuid).ok_or("Session not found")?;
        session.emotions.retain(|emotion| emotion.uuid != uuid);
        self.schedule_session_update();
        session.write();
      },
      SocketMessage::EditDescription(description) => {
        let patient_uuid = state.sessions.iter().find(|session| session.uuid == self.uuid).ok_or("Session not found")?.patient_uuid.clone();
        let patient = state.patients.iter_mut().find(|patient| patient.uuid == patient_uuid).ok_or("Patient not found")?;
        patient.description = description;
        self.schedule_patient_update();
        patient.write();
      },
    };
    Ok(())
  }
}

impl Actor for SessionSocket {
  type Context = ws::WebsocketContext<Self>;

  fn started(&mut self, ctx: &mut Self::Context) {
    info!("Session socket started");
    
    self.addr = Some(Arc::new(ctx.address()));
    self.hb(ctx);
  }

  fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
    info!("Session socket stopping");
    
    Running::Stop
  }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for SessionSocket {
  fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
    match item {
      Ok(ws::Message::Ping(msg)) => {
        self.hb = Instant::now();
        ctx.pong(&msg);
      },
      Ok(ws::Message::Pong(_)) => {
        self.hb = Instant::now();
      },
      Ok(ws::Message::Text(text)) => {
        let msg: String = text.into();

        if self.authorized.load(Ordering::Relaxed) {
          let addr = self.addr.clone().unwrap();
          let arc_state = Arc::new(self.clone());

          tokio::spawn(async move {
            if let Err(err) = arc_state.handle_messge(msg).await {
              addr.do_send(WsMessage(err.to_string()));
              addr.do_send(CloseSession);
              error!("Error sending message: {:?}", err);
            }
          });
          
          return;
        }
        
        let state = self.state.clone();
        let authorized = self.authorized.clone();
        let addr = self.addr.clone().unwrap();
        
        let id = self.socket_id;
        tokio::spawn(async move {
          let state = state.read().await;
          if state.users.contains_key(&msg) {
            info!("Session socket authorized");
            
            authorized.store(true, Ordering::Relaxed);
            addr.do_send(WsMessage(format!("Authorized: {}", id)));
            return;
          }

          info!("Session socket failed to authorize");
          addr.do_send(WsMessage("Unauthorized".to_string()));
          addr.do_send(CloseSession);
        });
      },
      Err(err) => {
        error!("Error handling message: {:?}", err);
        ctx.stop();
      },
      _ => (),
    }
  }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct WsMessage(pub String);

impl Handler<WsMessage> for SessionSocket {
  type Result = ();

  fn handle(&mut self, msg: WsMessage, ctx: &mut Self::Context) {
    ctx.text(msg.0);
  }
} 

#[derive(Message)]
#[rtype(result = "()")]
pub struct CloseSession;

impl Handler<CloseSession> for SessionSocket {
  type Result = ();

  fn handle(&mut self, _: CloseSession, ctx: &mut Self::Context) {
    ctx.stop();
  }
}
