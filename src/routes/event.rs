use crate::state::state::GoogleEvent;
use crate::filestreamer::FileStreamer;
use crate::AppState;
use crate::logs::*;

use std::fs;

use actix_web::{web, HttpRequest, HttpResponse};
use reqwest::ClientBuilder;
use serde_json::Deserializer;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resp {
  next_sync_token: String,
  items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletedEvent {
  resource_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Item {
  Event(GoogleEvent),
  Deleted(DeletedEvent),
}

#[actix_web::post("/events/webhook")]
pub async fn index(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
  let app_state = state.read().await;
  let id = req.headers().get("x-goog-resource-id").and_then(|id| id.to_str().inspect_err(|err| error!("Error: {}", err)).ok());
  let id = match id {
    Some(id) => id,
    None => {
      error!("Error: No id");
      return HttpResponse::BadRequest().finish();
    }
  };
  
  let webhook = match app_state.calendar_webhooks.iter().find(|(_, webhook)| webhook.resource_id == id) {
    Some(webhook) => webhook,
    None => {
      error!("Error: No webhook");
      return HttpResponse::BadRequest().finish();
    }
  };
  
  let auth = app_state.users.get(webhook.0).unwrap().read().await.access_token.clone();
  let sync_token = webhook.1.sync_token.clone();
  let user = webhook.0.clone();
  
  let client = ClientBuilder::new()
    .danger_accept_invalid_certs(true)
    .build()
    .unwrap();

  drop(app_state);
  let google_req = client.get("https://www.googleapis.com/calendar/v3/calendars/primary/events")
    .bearer_auth(auth)
    .query(&[("syncToken", sync_token.as_str()), ("maxResults", "2500")])
    .send()
    .await;

  let google_req = match google_req {
    Ok(google_req) => google_req,
    Err(err) => {
      error!("Error: {}", err);
      return HttpResponse::InternalServerError().finish();
    }
  };

  let google_req = match google_req.json::<Resp>().await {
    Ok(google_req) => google_req,
    Err(err) => {
      error!("Error: {}", err);
      return HttpResponse::InternalServerError().finish();
    }
  };

  let mut app_state = state.write().await;
  let webhook = app_state.calendar_webhooks.get_mut(&user).unwrap();
  webhook.sync_token = google_req.next_sync_token;

  info!("{:?}", google_req.items);

  // let file = fs::File::open(format!("{}events/{}.json", app_state.path, user)).unwrap();
  // let stream = Deserializer::from_reader(file).into_iter::<Vec<GoogleEvent>>();

  app_state.write();
  HttpResponse::NoContent().finish()
}

#[actix_web::get("/events")]
pub async fn get_events(req: HttpRequest, state: web::Data<AppState>) -> Result<HttpResponse, actix_web::Error> {
  let app_state = state.read().await;
  let user = app_state.auth_token(req)?;

  let file = match fs::File::open(format!("{}events/{}.json", app_state.path, user)) {
    Ok(file) => file,
    Err(_) => {
      return Ok(HttpResponse::Ok().body("[]"));
    }
  };

  let streamer = FileStreamer(file);
  Ok(HttpResponse::Ok().streaming(streamer))
}