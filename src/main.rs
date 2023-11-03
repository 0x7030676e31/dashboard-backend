#![feature(async_closure)]

use crate::state::state::State;

use std::sync::Arc;
use std::env;

use actix_web::{HttpServer, App};
use actix_web::web::Data;
use tokio::sync::{RwLock, mpsc};

mod state;
mod routes;
mod macros;
mod consts;

pub use macros::macros as logs;
pub type AppState = Arc<RwLock<State>>;
pub type User = Arc<RwLock<state::user::User>>;

pub fn get_url() -> String {
  #[cfg(not(production))]
  return format!("http://localhost:{}", env::var("PORT").unwrap_or("2137".into()));
  #[cfg(production)]
  return consts::URL.into();
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
  env::set_var("RUST_LOG", "INFO");
  env_logger::init();

  let port = env::var("PORT").map_or(2137, |port| port.parse().unwrap_or(2137));
  env::set_var("PORT", port.to_string());
  logs::info!("Starting server on port {}...", port);

  let (write_tx, write_rx) = mpsc::channel(1);
  let state = State::new(write_tx)?;
  let state = Arc::new(RwLock::new(state));

  State::start_write_loop(Arc::clone(&state), write_rx);

  HttpServer::new(move || {
    App::new()
      .wrap(actix_cors::Cors::permissive())
      .app_data(Data::new(state.clone()))
      .service(routes::get_routes())
  })
  .bind(("127.0.0.1", port))?
  .run()
  .await
}
