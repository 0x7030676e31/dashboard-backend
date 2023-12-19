#![feature(async_closure)]

use crate::state::state::State;

use std::sync::Arc;
use std::env;

use actix_web::{HttpServer, App, web, Responder};
use actix_web::web::Data;
use tokio::sync::{RwLock, mpsc};
use include_dir::{include_dir, Dir};

mod state;
mod routes;
mod macros;
mod consts;

pub use macros::macros as logs;
pub type AppState = Arc<RwLock<State>>;
pub type User = Arc<RwLock<state::user::User>>;

static DIST: Dir<'_> = include_dir!("./dist/assets");
static INDEX: &str = include_str!("../dist/index.html");

#[derive(Clone)]
pub struct EnvVars {
  is_production: bool,
  inner_port: u16,
  dev_port: u16,
  users: Vec<String>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
  dotenv::dotenv().ok();

  let is_production = env::var("PRODUCTION").map_or(false, |production| production == "true");
  let users = env::var("USERS").unwrap_or("".into()).split_whitespace().map(|s| s.to_owned()).collect::<Vec<_>>();

  let inner_port = env::var("INNER_PORT").map_or(2137, |port| port.parse().unwrap_or(2137));
  let dev_port = env::var("DEV_PORT").map_or(5173, |port| port.parse().unwrap_or(5173));

  let path = if is_production { "/root/".into() } else { env::var("FS").unwrap_or("/root/".into()) };

  let env_vars = EnvVars {
    is_production,
    inner_port,
    dev_port,
    users,
  };

  macros::first(is_production);
  env::set_var("RUST_LOG", "INFO");
  env_logger::init();

  let (write_tx, write_rx) = mpsc::channel(1);
  let state = State::new(write_tx, path)?;
  let state = Arc::new(RwLock::new(state));
  
  State::start_write_loop(Arc::clone(&state), write_rx);
  State::spawn_ping_loop(Arc::clone(&state));
  
  logs::info!("Starting server on inner port {}...", inner_port);

  HttpServer::new(move || {
    App::new()
    .wrap(actix_cors::Cors::permissive())
      .app_data(Data::new(state.clone()))
      .app_data(Data::new(env_vars.clone()))
      .route("/assets/{path:.*}", web::get().to(asset))
      .route("/", web::get().to(index))
      .default_service(web::get().to(index))
      .service(routes::get_routes())
  })
  .bind(("0.0.0.0", inner_port))?
  .run()
  .await
}

async fn asset(path: web::Path<String>) -> impl Responder {
  let path = path.into_inner();
  let asset = DIST.get_file(&path).expect(format!("Failed to get asset: {}", path).as_str()).contents();
  
  if !path.ends_with(".js") {
    return actix_web::HttpResponse::Ok().body(asset);
  }

  actix_web::HttpResponse::Ok()
    .content_type("application/javascript")
    .body(asset)
}

async fn index() -> impl Responder {
  actix_web::HttpResponse::Ok().body(INDEX)
}
