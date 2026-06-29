pub mod handlers;
pub mod middleware;
pub mod models;

// Re-export AuthUser from old auth.rs location
pub use crate::auth_simple::AuthUser;
