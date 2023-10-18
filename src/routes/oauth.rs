use actix_web::{HttpResponse, Responder};

#[actix_web::get("/authorize")]
pub async fn index() -> impl Responder {
  HttpResponse::Ok().body("Hello world!")
}