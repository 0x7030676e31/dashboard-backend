// Storage path
#[cfg(not(production))]
pub const PATH: &'static str = concat!(env!("HOME"), "/.config/dashboard");
#[cfg(production)]
pub const PATH: &'static str = "./";

// Site URL
#[cfg(not(production))]
pub const URL: &'static str = "http://localhost:2137";
#[cfg(production)]
pub const URL: &'static str = ""; // Todo: add production URL

// Google OAuth2 scope
pub const SCOPES: [&'static str; 2] = ["https://www.googleapis.com/auth/calendar", "https://www.googleapis.com/auth/userinfo.profile"];
