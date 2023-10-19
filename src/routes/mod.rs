use actix_web::{Scope, web};

mod oauth;

pub fn get_routes() -> Scope {
  web::scope("")
    .service(oauth::index)
    .service(oauth::oauth)
    .service(api())
}

fn api() -> Scope {
  web::scope("/api")
}