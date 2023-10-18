pub mod macros {
  macro_rules! info {
    ($($arg:tt)*) => {
      log::info!($($arg)*);
    }
  }

  macro_rules! debug {
    ($($arg:tt)*) => {
      log::debug!($($arg)*);
    }
  }

  macro_rules! error {
    ($($arg:tt)*) => {
      log::error!($($arg)*);
    }
  }

  pub(crate) use { info, debug, error };
}