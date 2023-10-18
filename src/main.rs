use crate::state::state::State;

use std::sync::Arc;
use std::env;

use actix_web::{HttpServer, App};
use actix_web::web::Data;
use tokio::sync::RwLock;

mod state;
mod routes;
mod macros;
mod consts;

pub use macros::macros as logs;
pub type AppState = Arc<RwLock<State>>;

#[tokio::main]
async fn main() -> std::io::Result<()> {
  env::set_var("RUST_LOG", "INFO");
  env_logger::init();

  let port = env::var("PORT").map_or(2137, |port| port.parse().unwrap_or(2137));
  logs::info!("Starting server on port {}...", port);
  let state = Arc::new(RwLock::new(State::new()?));

  HttpServer::new(move || {
    App::new()
      .app_data(Data::new(state.clone()))
      .service(routes::get_routes())
  })
  .bind(("127.0.0.1", port))?
  .run()
  .await
}
