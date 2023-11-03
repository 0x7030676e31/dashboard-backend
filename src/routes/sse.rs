use crate::AppState;

use actix_web::{web, HttpResponse};

#[actix_web::get("/token")]
pub async fn new_token(state: web::Data<AppState>) -> HttpResponse {
  HttpResponse::Ok().body("Hello world!")
}