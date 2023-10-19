#[allow(clippy::module_inception)]
pub mod macros {
  macro_rules! info {
    ($($arg:tt)*) => {
      log::info!($($arg)*);
    }
  }

  macro_rules! warning {
    ($($arg:tt)*) => {
      log::warn!($($arg)*);
    }
  }

  macro_rules! error {
    ($($arg:tt)*) => {
      log::error!($($arg)*);
    }
  }

  pub(crate) use { info, warning, error };
}