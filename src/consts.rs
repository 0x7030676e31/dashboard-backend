// Storage path
#[cfg(not(production))]
pub const PATH: &str = concat!(env!("HOME"), "/.config/dashboard");
#[cfg(production)]
pub const PATH: &str = "./";

// Site URL
#[cfg(not(production))]
pub const URL: &str = "http://localhost:5173";
#[cfg(production)]
pub const URL: &str = ""; // Todo: add production URL

// Google OAuth2 scope
pub const SCOPES: [&str; 3] = [
  "https://www.googleapis.com/auth/calendar",
  "https://www.googleapis.com/auth/userinfo.profile",
  "https://www.googleapis.com/auth/userinfo.email"
];

// Permitted users
pub const USERS: [&str; 2] = ["t.chmielewski22@zsek.olszty.pl", "p0gn10wn14@gmail.com"];