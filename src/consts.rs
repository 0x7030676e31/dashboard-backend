// Storage path
#[cfg(not(production))]
pub const PATH: &'static str = concat!(env!("HOME"), "/.config/dashboard");
#[cfg(production)]
pub const PATH: &'static str = "./";

// Google OAuth2 redirect URI
#[cfg(not(production))]
pub const REDIRECT: &'static str = "http://localhost:2137/oauth";
#[cfg(production)]
pub const REDIRECT: &'static str = ""; // Todo: add production redirect URI

// Google OAuth2 scope
pub const SCOPE: &'static str = "https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/userinfo.profile";