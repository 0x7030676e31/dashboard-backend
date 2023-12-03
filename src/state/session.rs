use crate::{logs::*, AppState, consts};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emotion {
  pub uuid: String,
  pub id: Option<u8>,
  pub kind: Option<EmotionType>,
  pub aquired_age: Option<u8>,
  pub aquired_person: String,
  pub created_at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum EmotionType {
  Acquired,
  Inherited,
  Owned,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum TimelineEvent {
  SessionStart,
  SessionEnd,
  EmotionAdded(String),
  EmotionRemoved(String),
}

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

    info!("Loaded {} sessions", sessions.len());
    Ok(sessions)
  }

  pub fn write(&self) {
    let path = format!("{}/sessions/{}.json", consts::PATH, self.uuid);
    if let Err(err) = fs::write(path, serde_json::to_string(self).unwrap()) {
      error!("Couldn't write session to file: {}", err);
    }
  }

  pub fn delete(&self) {
    let path = format!("{}/sessions/{}.json", consts::PATH, self.uuid);
    if let Err(err) = fs::remove_file(path) {
      error!("Couldn't delete session file: {}", err);
    }
  }
}

#[derive(Debug, Clone)]
pub struct SessionSocket {
  state: AppState,
  uuid: String,
  authorized: Arc<AtomicBool>,
  addr: Option<Arc<Addr<SessionSocket>>>,
  hb: Instant,
}

#[derive(Deserialize)]
struct EditEmotion {
  uuid: String,
  id: Option<u8>,
  kind: Option<EmotionType>,
  aquired_age: Option<u8>,
  aquired_person: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "payload")]
enum SocketMessage {
  AddEmotion(String),
  EditEmotion(EditEmotion),
  RemoveEmotion(String),
  EditDescription(String),
}

impl SessionSocket {
  pub fn new(state: AppState, uuid: String) -> Self {
    Self {
      state,
      uuid,
      authorized: Arc::new(AtomicBool::new(false)),
      addr: None,
      hb: Instant::now(),
    }
  }
  
  fn hb(&self, ctx: &mut ws::WebsocketContext<Self>) {
    ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
      if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
        println!("Disconnecting failed heartbeat");
        ctx.stop();
        return;
      }

      ctx.ping(b"hi");
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

        session.write();
      },
      _ => (),
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
        
        tokio::spawn(async move {
          let state = state.read().await;
          if state.users.contains_key(&msg) {
            info!("Session socket authorized");
            
            authorized.store(true, Ordering::Relaxed);
            addr.do_send(WsMessage("Authorized".to_string()));
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

  fn handle(&mut self, _msg: CloseSession, ctx: &mut Self::Context) {
    ctx.stop();
  }
}
