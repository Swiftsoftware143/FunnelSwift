use crate::state::AppState;
use axum::{routing, Router};

pub mod handlers;

pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/",
            routing::get(handlers::list_workspaces).post(handlers::create_workspace),
        )
        .route(
            "/:id",
            routing::patch(handlers::rename_workspace).delete(handlers::delete_workspace),
        )
        .route(
            "/:id/default",
            routing::post(handlers::set_default_workspace),
        )
        .route("/:id/switch", routing::post(handlers::switch_workspace))
}
