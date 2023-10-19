// Storage path
#[cfg(not(production))]
pub const PATH: &str = concat!(env!("HOME"), "/.config/dashboard");
#[cfg(production)]
pub const PATH: &str = "./";

// Site URL
#[cfg(not(production))]
pub const URL: &str = "http://localhost:2137";
#[cfg(production)]
pub const URL: &str = ""; // Todo: add production URL

// Google OAuth2 scope
pub const SCOPES: [&str; 2] = ["https://www.googleapis.com/auth/calendar", "https://www.googleapis.com/auth/userinfo.profile"];
