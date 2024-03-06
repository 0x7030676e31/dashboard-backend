use crate::AppState;
use crate::logs::*;

use std::collections::HashMap;

use actix_web::{web, HttpRequest, HttpResponse};

#[actix_web::post("/event")]
pub async fn index(req: HttpRequest, _state: web::Data<AppState>, body: web::Payload) -> HttpResponse {
  let body = body.to_bytes().await.unwrap().to_vec().into_iter().map(|byte| byte as char).collect::<String>();
  
  let headers = req.headers().iter().map(|(key, value)| (key.to_string(), value.to_str().unwrap().to_string())).collect::<HashMap<String, String>>();
  let headers = serde_json::to_string(&headers).unwrap();

  info!("{}", body);
  info!("{}", headers);

  HttpResponse::NoContent().finish()
}