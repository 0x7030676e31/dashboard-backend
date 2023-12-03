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

pub struct SessionSocket {
  state: AppState,
  authorized: Arc<AtomicBool>,
  addr: Option<Arc<Addr<SessionSocket>>>,
  hb: Instant,
}

impl SessionSocket {
  pub fn new(state: AppState) -> Self {
    Self {
      state,
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
}

impl Actor for SessionSocket {
  type Context = ws::WebsocketContext<Self>;

  fn started(&mut self, ctx: &mut Self::Context) {
    println!("Socket started");
    
    self.addr = Some(Arc::new(ctx.address()));
    self.hb(ctx);
  }

  fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
    println!("Socket stopping");
    
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

        if !self.authorized.load(Ordering::Relaxed) {
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
          return;
        }
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