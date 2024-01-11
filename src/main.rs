#![feature(async_closure, let_chains)]

use crate::state::state::State;

use std::sync::Arc;
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::env;

use actix_web::{HttpServer, App, web, Responder};
use actix_web::web::Data;
use tokio::sync::{RwLock, mpsc};
use include_dir::{include_dir, Dir};
use rustls::{ServerConfig, Certificate, PrivateKey};
use rustls::server::{ResolvesServerCert, ClientHello};
use rustls::sign::{CertifiedKey, any_supported_type};

mod state;
mod routes;
mod macros;
mod consts;
mod google;

pub use macros::macros as logs;
pub type AppState = Arc<RwLock<State>>;
pub type User = Arc<RwLock<state::user::User>>;

static DIST: Dir<'_> = include_dir!("./dist/assets");
static INDEX: &str = include_str!("../dist/index.html");

struct MultiDomainResolver (HashMap<String, Arc<CertifiedKey>>);

impl ResolvesServerCert for MultiDomainResolver {
  fn resolve(&self, dns_name: ClientHello) -> Option<Arc<CertifiedKey>> {
    dns_name.server_name().and_then(|name| self.0.get(name).cloned())
  }
}

const DOMAIN: &str = "entitia";
impl MultiDomainResolver {
  pub fn add_cert(&mut self, path: &String, top_domain: &str) -> io::Result<()> {
    let cert = format!("{}{}/certificate.pem", path, top_domain);
    let key = format!("{}{}/private.pem", path, top_domain);

    let cert_file = File::open(cert)?;
    let mut cert_reader = io::BufReader::new(cert_file);
    let certs = rustls_pemfile::certs(&mut cert_reader)
      .map(|cert|cert.map(|cert| Certificate(cert.into_iter().map(|byte| byte.to_owned()).collect())))
      .collect::<Result<Vec<_>, _>>()?;

    let key_file = File::open(key)?;
    let mut key_reader = io::BufReader::new(key_file);
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader).collect::<Result<Vec<_>, _>>()?;

    let key = match keys.len() {
      1 => PrivateKey(keys.remove(0).secret_pkcs8_der().to_vec()),
      0 => return Err(io::Error::new(io::ErrorKind::InvalidInput, "No keys found")),
      _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "Multiple keys found")),
    };

    let key = match any_supported_type(&key) {
      Ok(key) => key,
      Err(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid key type")),
    };

    let cert = CertifiedKey::new(certs, key);
    self.0.insert(format!("{}.{}", DOMAIN, top_domain), Arc::new(cert));

    Ok(())
  }
}

#[derive(Clone)]
pub struct EnvVars {
  is_production: bool,
  inner_port: u16,
  dev_port: u16,
  users: Vec<String>,
}

#[tokio::main]
async fn main() -> io::Result<()> {
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
  let state = State::new(write_tx, path.clone())?;
  let state = Arc::new(RwLock::new(state));
  
  State::start_write_loop(Arc::clone(&state), write_rx);
  State::spawn_ping_loop(Arc::clone(&state));
  
  logs::info!("Starting server on inner port {}...", inner_port);

  let server = HttpServer::new(move || {
    App::new()
    .wrap(actix_cors::Cors::permissive())
      .app_data(Data::new(state.clone()))
      .app_data(Data::new(env_vars.clone()))
      .route("/assets/{path:.*}", web::get().to(asset))
      .route("/", web::get().to(index))
      .default_service(web::get().to(index))
      .service(routes::get_routes())
  });

  if !is_production {
    return server
      .bind(("0.0.0.0", inner_port))?
      .run()
      .await
  }

  let mut resolver = MultiDomainResolver(HashMap::new());
  resolver.add_cert(&path, "com")?;
  resolver.add_cert(&path, "pl")?;

  let cfg = ServerConfig::builder()
    .with_safe_defaults()
    .with_no_client_auth()
    .with_cert_resolver(Arc::new(resolver));
  
  server
    .bind_rustls_021(format!("0.0.0.0:{}", inner_port), cfg)?
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
