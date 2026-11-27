pub mod global_auth;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod route_policy;
pub mod session_provenance;

// The fleet standard's §6 guard test for account closure / data erasure (test-only).
#[cfg(test)]
mod erasure_guard;

// Re-export AuthUser from old auth.rs location
