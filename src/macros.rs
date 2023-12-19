use std::sync::{OnceLock, Mutex};
use std::collections::VecDeque;
use std::iter::once;
use std::env;
use std::fs;

pub fn path() -> &'static String {
  static PATH: OnceLock<String> = OnceLock::new();
  PATH.get_or_init(|| {
    let is_prod = env::var("PRODUCTION").map_or(false, |production| production == "true");
    if is_prod { "/root/".into() } else { env::var("FS").unwrap_or("/root/".into()) }
  })
}

fn lines() -> &'static Mutex<VecDeque<String>> {
  static LINES: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
  LINES.get_or_init(|| {
    let path = format!("{}logs.txt", path());
    match fs::read_to_string(path.as_str()) {
      Ok(s) => Mutex::new(s.trim().split('\n').map(|s| s.to_owned()).collect::<VecDeque<_>>()),
      Err(_) => Mutex::new(VecDeque::new()),
    }
  })
}

const SIZE: usize = 1024;
pub fn write(level: impl AsRef<str>, source_file: impl AsRef<str>, line: impl AsRef<str>) {
  let date = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
  let line = format!("[{: <7} {}] {: <25} {}", level.as_ref(), date, &source_file.as_ref()[4..], line.as_ref());
  let path = format!("{}logs.txt", path());

  let mut lines = lines().lock().unwrap();
  lines.push_back(line);

  if lines.len() > SIZE {
    lines.pop_front();
  }

  fs::write(path.as_str(), lines.iter().chain(once(&"".into())).map(|s| s.as_str()).collect::<Vec<_>>().join("\n")).unwrap();
}

pub fn first(is_prod: bool) {
  let date = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
  let line = format!("========================[ RUNNING IN {}; {} ]========================", if is_prod { "PRODUCTION" } else { "DEVELOPMENT" }, date);
  let path = format!("{}logs.txt", path());
  
  let mut lines = lines().lock().unwrap();
  if lines.len() > 0 {
    lines.push_back("".into());
  }

  lines.push_back(line);
  lines.push_back("".into());

  while lines.len() > SIZE {
    lines.pop_front();
  }

  fs::write(path.as_str(), lines.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")).unwrap();
}

#[allow(clippy::module_inception)]
pub mod macros {
  macro_rules! fspath {
    () => {
      crate::macros::path()
    };
  }

  macro_rules! info {
    ($($arg:tt)*) => {
      log::info!($($arg)*);
      crate::macros::write("INFO", file!(), format!($($arg)*));
    }
  }

  macro_rules! warning {
    ($($arg:tt)*) => {
      log::warn!($($arg)*);
      crate::macros::write("WARN", file!(), format!($($arg)*));
    }
  }

  macro_rules! error {
    ($($arg:tt)*) => {
      log::error!($($arg)*);
      crate::macros::write("ERROR", file!(), format!($($arg)*));
    }
  }

  pub(crate) use { fspath, info, warning, error };
}