use crate::macros::path;
use crate::state::session::{SessionSocket, Session, Emotion};
use crate::{AppState, google};
use crate::state::state::SseEvent;
use crate::logs::*;

use std::collections::HashMap;
use std::sync::Arc;
use std::{fs, env};
use std::time::Instant;

use actix_web::delete;
use actix_web::{HttpRequest, web, Error, HttpResponse, get, post, patch};
use actix_web_actors::ws;
use chrono::Datelike;
use futures_util::future;
use serde::Deserialize;
use headless_chrome::types::PrintToPdfOptions;
use headless_chrome::{Browser, LaunchOptions};
use uuid::Uuid;

const TEMPLATE: &str = include_str!("../../template.html");

#[derive(Deserialize)]
struct NewPatient {
  patient: String,
  time_start: u64,
  time_end: u64,
}

#[post("/")]
pub async fn create_session(req: HttpRequest, state: web::Data<AppState>, new_session: web::Json<NewPatient>) -> Result<HttpResponse, actix_web::Error> {
  let mut app_state = state.write().await;
  app_state.auth_token(req)?;

  if !app_state.patients.iter().any(|patient| patient.uuid == new_session.patient) {
    return Ok(HttpResponse::NotFound().body("Patient not found"));
  }

  let NewPatient { patient, time_start, time_end } = new_session.into_inner();
  info!("Created session for patient {}", patient);

  let emotion_uuid = Uuid::new_v4();
  let uuid = Uuid::new_v4();

  let session = Session {
    uuid: uuid.to_string(),
    patient_uuid: patient,
    start: time_start,
    end: time_end,
    paid: 0.0,
    emotions: vec![Emotion {
      uuid: emotion_uuid.to_string(),
      id: None,
      kind: None,
      aquired_age: None,
      aquired_person: String::new(),
      created_at: chrono::Utc::now().timestamp() as u64,
    }],
    timeline: HashMap::new(),
    created_at: chrono::Utc::now().timestamp() as u64,
    last_updated: chrono::Utc::now().timestamp() as u64,
    calendar_ids: HashMap::new(),
  };

  let patient = app_state.patients.iter().find(|patient| patient.uuid == session.patient_uuid).unwrap();
  let raw_calendar_event = google::RawCalendarEvent {
    start: time_start,
    end: time_end,
    description: Some(patient.description.to_owned()),
    summary: format!("S. {}", if patient.name.is_empty() { "<Pacjent bez nazwy>" } else { patient.name.as_str() }),
    uuid: uuid.to_string(),
    colorId: None,
  };

  session.write();
  app_state.broadcast(SseEvent::SessionAdded(&session)).await;
  app_state.sessions.push(session);

  let app_state = state.clone();
  tokio::spawn(async move {
    let mut app_state = app_state.write().await;
    let mut ids = HashMap::new();
    
    for user in app_state.users.values() {      
      let user = user.read().await;
      if !user.settings.google_calendar_enabled {
        continue;
      }

      match google::add_event(&user.access_token, &raw_calendar_event).await {
        Ok(id) => { ids.insert(user.user_info.email.clone(), id); },
        Err(err) => { error!("Failed to add event for user {}: {}", user.user_info.email, err); },
      };
    }

    let session = app_state.sessions.iter_mut().find(|s| s.uuid == uuid.to_string()).unwrap();
    session.calendar_ids = ids;

    session.write();
    info!("Added events for session {}", uuid);
  });
  
  Ok(HttpResponse::Ok().body(uuid.to_string()))
}

#[derive(Deserialize)]
struct UpdateSession {
  patient: Option<String>,
  time_start: Option<u64>,
  time_end: Option<u64>,
  paid: Option<f32>,
}

#[patch("/{uuid}")]
pub async fn update_session(req: HttpRequest, state: web::Data<AppState>, payload: web::Json<UpdateSession>, session_uuid: web::Path<String>) -> Result<HttpResponse, Error> {
  let mut app_state = state.write().await;
  app_state.auth_token(req)?;

  let UpdateSession { patient, time_start, time_end, paid } = payload.into_inner();
  if patient.is_some() && !app_state.patients.iter().any(|pt| patient.as_ref().is_some_and(|p| p == &pt.uuid)) {
    return Ok(HttpResponse::NotFound().body("Patient not found"));
  }

  let session_uuid = session_uuid.into_inner();
  let session = match app_state.sessions.iter_mut().find(|s| s.uuid == session_uuid) {
    Some(session) => session,
    None => return Ok(HttpResponse::NotFound().body("Not Found")),
  };

  let mut do_update = false;
  if let Some(patient) = patient && patient != session.patient_uuid {
    session.patient_uuid = patient;
    do_update = true;
  }

  if let Some(time_start) = time_start && time_start != session.start {
    session.start = time_start;
    do_update = true;
  }

  if let Some(time_end) = time_end && time_end != session.end {
    session.end = time_end;
    do_update = true;
  }

  if do_update {
    let state2 = state.clone();
    let ids = session.calendar_ids.clone();
    let start = session.start;
    let end = session.end;
    let patient_uuid = session.patient_uuid.clone();
    tokio::spawn(async move {
      let state = state2.read().await;
    
      let patient = state.patients.iter().find(|p| p.uuid == patient_uuid).unwrap();
      let summary = format!("S. {}", patient.name);
    
      let users = state.users.values().map(|u| u.read());
      let users = future::join_all(users).await;

      for (email, id) in ids {
        let user = users.iter().find(|u| u.user_info.email == email).unwrap();
        let event_edit = google::EditEvent {
          start,
          end,
          description: Some(patient.description.to_owned()),
          summary: summary.clone(),
          id,
          colorId: None,
        };
        
        match google::edit_event(&user.access_token, &event_edit).await {
          Ok(_) => {},
          Err(err) => { error!("Failed to edit event for user {}: {}", user.user_info.email, err); },
        };
      }

      info!("Edited events for session {}", session_uuid);
    });
  }

  if let Some(paid) = paid {
    session.paid = paid;
  }

  session.last_updated = chrono::Utc::now().timestamp() as u64;
  session.write();

  let session = session.clone();
  app_state.broadcast(SseEvent::SessionUpdated(&session)).await;

  info!("Updated session {}", session.uuid);
  Ok(HttpResponse::Ok().body("Updated"))
}

#[delete("/{uuid}")]
pub async fn delete_session(req: HttpRequest, state: web::Data<AppState>, session: web::Path<String>) -> Result<HttpResponse, Error> {
  let mut app_state = state.write().await;
  app_state.auth_token(req)?;

  let uuid = session.into_inner();
  let session = match app_state.sessions.iter().find(|s| s.uuid == uuid) {
    Some(session) => session,
    None => return Ok(HttpResponse::NotFound().body("Not Found")),
  };

  let calendar_ids = session.calendar_ids.clone();
  let state = state.clone();
  let uuid2 = uuid.clone();
  tokio::spawn(async move {
    let state = state.write().await;
    let users = state.users.values().map(|u| u.read());
    let users = future::join_all(users).await;
    
    let len = calendar_ids.len();
    for (email, id) in calendar_ids {
      let user = users.iter().find(|u| u.user_info.email == email).unwrap();
      match google::delete_event(&user.access_token, &id).await {
        Ok(_) => {},
        Err(err) => { error!("Failed to delete event for user {}: {}", user.user_info.email, err); },
      };
    }

    info!("Deleted {} events for session {}", len, uuid2);
  });

  session.delete();
  app_state.broadcast(SseEvent::SessionRemoved(&uuid)).await;
  app_state.sessions.retain(|s| s.uuid != uuid);

  info!("Deleted session {}", uuid);
  Ok(HttpResponse::Ok().body("Deleted"))
}

#[get("/{session}/stream")]
pub async fn stream(req: HttpRequest, state: web::Data<AppState>, payload: web::Payload, session: web::Path<String>) -> Result<HttpResponse, Error> {
  let session = session.into_inner();
  let app_state = state.clone();
  let app_state = app_state.read().await;
  
  if !app_state.sessions.iter().any(|s| s.uuid == session) {
    return Ok(HttpResponse::NotFound().body("Not Found"));
  }

  let socket = SessionSocket::new(Arc::clone(&*state.into_inner()), session);
  let resp = ws::start(socket, &req, payload)?;
  Ok(resp)
}

fn get_emotion_code(id: u8) -> u8 {
  id % 30 / 5
}

const TABLE: [&str; 60] = [
  "NIEODWZAJEMNIONA MIŁOŚĆ",
  "OPUSZCZENIE",
  "PORZUCENIE",
  "ZAGUBIENIE",
  "ZDRADA",
  "MARTWIENIE SIĘ",
  "NIEPOKÓJ",
  "ODRAZA",
  "ROZPACZ",
  "ZDENERWOWANIE",
  "ODRZUCENIE",
  "PŁACZ",
  "SMUTEK",
  "ZNIECHĘCENIE",
  "ŻAŁOŚĆ",
  "NIENAWIŚĆ",
  "POCZUCIE WINY",
  "ROZGORYCZENIE",
  "URAZA",
  "ZŁOŚĆ",
  "GROZA",
  "OBWINIANIE",
  "PRZERAŻENIE",
  "STRACH",
  "ZIRYTOWANIE",
  "PONIŻENIE",
  "POŻĄDANIE",
  "PRZYTŁOCZENIE",
  "TĘSKNOTA",
  "ZAZDROŚĆ",
  "EUFORIA",
  "NIEDOCENIENIE",
  "NIEPEWNOŚĆ",
  "WRAŻLIWOŚĆ",
  "ZŁAMANE SERCE",
  "BEZNADZIEJA",
  "BEZSILNOŚĆ",
  "BRAK KONTROLI",
  "NISKA SAMOOCENA",
  "PORAŻKA",
  "DEFENSYWNOŚĆ",
  "SAMOKRZYWDZENIE",
  "ZACIĘTOŚĆ",
  "ZDEZORIETOWANIE",
  "ŻAL",
  "BRAK UZNANIA",
  "DEPRESJA",
  "FRUSTRACJA",
  "NIEZDECYDOWANIE",
  "PANIKA",
  "BRAK WSPARCIA",
  "BYCIE BEZ WYRAZU",
  "KONFLIKTOWOŚĆ",
  "NIEPEWNOŚĆ TWORZENIA",
  "BYCIE ZASTRASZONYM",
  "BYCIE BEZWARTOŚCIOWYM",
  "BYCIE NIEGODNYM",
  "DUMA",
  "WSTYD",
  "WSTRZĄS",
];

const KIND_TABLE: [&str; 4] = [
  "WŁASNA",
  "ODZIEDZICZONA",
  "NABYTA",
  "PRENATALNA",
];

#[post("/{session}/pdf")]
pub async fn gen_pdf(req: HttpRequest, state: web::Data<AppState>, session: web::Path<String>) -> Result<HttpResponse, Error> {
  let session = session.into_inner();
  let app_state = state.clone();
  let app_state = app_state.read().await;

  app_state.auth_token(req)?;
  let session = match app_state.sessions.iter().find(|s| s.uuid == session) {
    Some(session) => session,
    None => return Ok(HttpResponse::NotFound().body("Not Found")),
  };

  let patient = match app_state.patients.iter().find(|p| p.uuid == session.patient_uuid) {
    Some(patient) => patient,
    None => return Ok(HttpResponse::NotFound().body("Patient not found")),
  };

  info!("Generating PDF for session {}", session.uuid);
  let now = Instant::now();

  let session_date = chrono::NaiveDateTime::from_timestamp_opt(session.start as i64, 0).unwrap();
  let mut percentages = [0; 6];
  
  let mut idx = 0;
  let emotions = session.emotions.iter().rev().filter_map(|emotion| emotion.id.map(|id| {
    let code = get_emotion_code(id);
    percentages[code as usize] += 1;
    idx += 1;
    format!(
      "<tr class=\"_{}\"><td>{}. {}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
      code + 1,
      idx,
      TABLE[id as usize],
      emotion.kind.map_or("—", |kind| KIND_TABLE[kind as usize]),
      emotion.aquired_age.map_or("—".to_string(), |age| age.to_string()),
      emotion.aquired_person.is_empty().then(|| "—").unwrap_or(&emotion.aquired_person),
    )
  })).collect::<Vec<_>>();

  let sum = percentages.iter().sum::<i32>() as f32;
  let mut percentages = percentages.iter().map(|&count| (count as f32 / sum * 100.0) as u8).collect::<Vec<_>>();
  if !emotions.is_empty() {
    percentages[5] = 100 - percentages.iter().take(5).sum::<u8>();
  }

  let percentages = percentages.iter().enumerate().map(|(idx, perc)| format!(
    "<td class=\"_{}\">{}%</td>",
    idx + 1,
    perc,
  )).collect::<Vec<_>>();

  let html = TEMPLATE
    .replace("{{name}}", &patient.name)
    .replace("{{dd}}", &session_date.day().to_string())
    .replace("{{mm}}", &session_date.month().to_string())
    .replace("{{yy}}", &session_date.year().to_string())
    .replace("{{emotions}}", &emotions.join(""))
    .replace("{{percentages}}", &percentages.join(""));

  let session_uuid = session.uuid.clone();
  drop(app_state);

  let uuid = Uuid::new_v4();
  fs::write(format!("{}pdf/{}.html", path(), uuid), html).unwrap();

  let opt = LaunchOptions {
    headless: true,
    sandbox: false,
    ..Default::default()
  };

  let browser = match Browser::new(opt) {
    Ok(browser) => browser,
    Err(err) => {
      error!("Failed to launch browser for session {}: {}", session_uuid, err);
      return Ok(HttpResponse::InternalServerError().finish());
    },
  };

  let tab = match browser.new_tab() {
    Ok(tab) => tab,
    Err(err) => {
      error!("Failed to create tab for session {}: {}", session_uuid, err);
      return Ok(HttpResponse::InternalServerError().finish());
    },
  };

  let opt = PrintToPdfOptions {
    landscape: Some(false),
    display_header_footer: Some(false),
    print_background: Some(true),
    scale: Some(1.0),
    paper_width: Some(8.27),
    paper_height: Some(11.7),
    margin_top: Some(0.0),
    margin_bottom: Some(0.0),
    margin_left: Some(0.0),
    margin_right: Some(0.0),
    page_ranges: None,
    ignore_invalid_page_ranges: None,
    header_template: None,
    footer_template: None,
    prefer_css_page_size: None,
    transfer_mode: None,
  };

  let is_production = env::var("PRODUCTION").map_or(false, |production| production == "true");
  let url = if is_production { format!("https://entitia.com/{}/html", uuid) } else { format!("http://localhost:{}/{}/html", env::var("INNER_PORT").unwrap_or("2137".into()), uuid) };

  let tab = match tab.navigate_to(&url) {
    Ok(res) => res,
    Err(err) => {
      error!("Failed to navigate to file:// for session {}: {}", session_uuid, err);
      return Ok(HttpResponse::InternalServerError().finish());
    },
  };

  let tab = match tab.wait_until_navigated() {
    Ok(res) => res,
    Err(err) => {
      error!("Failed to wait until navigated for session {}: {}", session_uuid, err);
      return Ok(HttpResponse::InternalServerError().finish());
    },
  };

  let bytes = match tab.print_to_pdf(Some(opt)) {
    Ok(bytes) => bytes,
    Err(err) => {
      error!("Failed to generate PDF for session {}: {}", session_uuid, err);
      return Ok(HttpResponse::InternalServerError().finish());
    },
  };

  fs::write(format!("{}pdf/{}.pdf", path(), uuid), bytes).unwrap();
  fs::remove_file(format!("{}pdf/{}.html", path(), uuid)).unwrap();

  info!("Generated PDF for session {}, took {}ms", session_uuid, now.elapsed().as_millis());

  Ok(HttpResponse::Ok().body(uuid.to_string()))
}
