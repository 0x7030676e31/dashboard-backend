use crate::logs::*;

use std::sync::OnceLock;
use std::error::Error;

use chrono::{TimeZone, Utc};
use reqwest::{Client, ClientBuilder};
use serde::{Serialize, Deserialize};

fn client() -> &'static Client {
  static CLIENT: OnceLock<Client> = OnceLock::new();
  CLIENT.get_or_init(||
    ClientBuilder::new()
      .danger_accept_invalid_certs(true)
      .build()
      .unwrap()
  )
}

pub struct RawCalendarEvent {
  pub start: u64,
  pub end: u64,
  pub description: Option<String>,
  pub summary: String,
  pub uuid: String,
}

#[derive(Serialize, Debug)]
#[allow(non_snake_case)]
struct Time {
  dateTime: String,
  timeZone: String,
}

#[derive(Deserialize, Debug)]
struct ErrorResponse {
  error: ErrorBody,
}

#[derive(Deserialize, Debug)]
struct ErrorBody {
  code: u16,
  message: String,
}

impl From<u64> for Time {
  fn from(time: u64) -> Self {
    let utc = Utc.timestamp_millis_opt(time as i64  * 1000).unwrap();
    Time { dateTime: utc.to_rfc3339(), timeZone: "Europe/Warsaw".into() }
  }
}

#[derive(Serialize, Debug)]
struct CreateEvent<'a> {
  start: &'a Time,
  end: &'a Time,
  description: &'a Option<String>,
  summary: &'a String,
  id: &'a String,
}


pub async fn add_events(auth: &String, events: &[&RawCalendarEvent]) -> Result<Vec<(String, String)>, Box<dyn Error>> {
  let mut result = Vec::with_capacity(events.len());
  
  let batches = events.chunks(50);
  for batch in batches {
    let resp = add_events_batch(auth, batch).await?;
    result.extend(resp);
  }

  Ok(result)
}

async fn add_events_batch(auth: &String, events: &[&RawCalendarEvent]) -> Result<Vec<(String, String)>, Box<dyn Error>> {
  let mut batch = String::new();
  let mut ids = Vec::with_capacity(events.len());

  for (idx, event) in events.iter().enumerate() {
    let event_id = Utc::now().timestamp_micros();
    let event_id = format!("{:x}", event_id);

    ids.push((event.uuid.clone(), event_id.clone()));
    let event = CreateEvent {
      start: &event.start.into(),
      end: &event.end.into(),
      description: &event.description,
      summary: &event.summary,
      id: &event_id,
    };

    let event = serde_json::to_string(&event)?;
    batch.push_str(&format!(
      "--batch_add_boundary\r\n\
      Content-Type: application/http\r\n\
      Content-ID: <item{}>\r\n\
      \r\n\
      POST /calendar/v3/calendars/primary/events\r\n\
      Content-Type: application/json\r\n\
      \r\n\r\n\r\n\
      {}\
      \r\n\r\n",
      idx + 1,
      event,
    ));
  }

  batch.push_str("--batch_add_boundary--");
  let resp = client()
    .post("https://www.googleapis.com/batch/calendar/v3")
    .bearer_auth(auth)
    .header("Content-Type", "multipart/mixed; boundary=batch_add_boundary")
    .body(batch)
    .send()
    .await?;

  let text = resp.text().await?;
  let resp_codes = parse_multipart_body(&text);

  for code in resp_codes {
    if code != 200 {
      error!("Failed to add event: {}", code);
      return Err("Failed to add event".into());
    }
  }

  Ok(ids)
}

pub async fn add_event(auth: &String, event: &RawCalendarEvent) -> Result<String, Box<dyn Error>> {
  let event_id = Utc::now().timestamp_micros();
  let event_id = format!("{:x}", event_id);

  let event = CreateEvent {
    start: &event.start.into(),
    end: &event.end.into(),
    description: &event.description,
    summary: &event.summary,
    id: &event_id,
  };

  let event = serde_json::to_string(&event)?;
  let resp = client()
    .post("https://www.googleapis.com/calendar/v3/calendars/primary/events")
    .bearer_auth(auth)
    .header("Content-Type", "application/json")
    .body(event)
    .send()
    .await?;

  if resp.status().is_success() {
    return Ok(event_id);
  }

  let text = resp.text().await?;
  let error: ErrorResponse = serde_json::from_str(&text)?;
  error!("Failed to add event: {}, {}", error.error.message, error.error.code);
  
  Err(error.error.message.into())
}

fn parse_multipart_body(body: &String) -> Vec<u16> {
  let lines = body.split("\\r\\n");
  let mut codes: Vec<u16> = Vec::new();
  for line in lines {
    if line.starts_with("HTTP/1.1") {
      let code = line.split_whitespace().nth(1).unwrap().parse::<u16>().unwrap();
      codes.push(code);
    }
  }

  codes
}