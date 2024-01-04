use actix_web::{Scope, web};

mod session;
mod patient;
mod oauth;
mod sse;
mod settings;

pub fn get_routes() -> Scope {
  web::scope("")
    .service(oauth::index)
    .service(oauth::oauth)
    .service(api())
}

fn api() -> Scope {
  web::scope("/api")
    .service(oauth::auth)
    .service(sse::get_token)
    .service(sse::stream)
    .service(sessions())
    .service(patients())
    .service(settings::index)
    .service(settings::google_calendar_resync)
}

fn sessions() -> Scope {
  web::scope("/sessions")
    .service(session::create_session)
    .service(session::update_session)
    .service(session::delete_session)
    .service(session::stream)
}

fn patients() -> Scope {
  web::scope("/patients")
    .service(patient::create_patient)
    .service(patient::update_patient)
    .service(patient::delete_patient)
}