use axum::{
    routing::{delete, get, post, put},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::auth::handlers::{login, me, register, change_password, forgot_password, reset_password};
use crate::handlers::{
    affiliate_handler, api_key_handler, dashboard_handler, lead_handler, plan_handler,
    plan_tag_handler, routing_handler, settings_handler, tag_group_handler, tag_handler,
    webhook_handler, portfolio_handler, integration_target_handler,
};
use crate::state::AppState;

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "funnelswift",
        "version": "0.1.0"
    }))
}

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/forgot-password", post(forgot_password))
        .route("/api/v1/auth/reset-password", post(reset_password))
        .route("/api/v1/auth/me", get(me))
        .route("/api/v1/auth/password", put(change_password))
        .route("/api/v1/leads", get(lead_handler::list_leads).post(lead_handler::create_lead))
        .route("/api/v1/leads/export", get(lead_handler::export_leads))
        .route("/api/v1/leads/:id", get(lead_handler::get_lead).put(lead_handler::update_lead).delete(lead_handler::delete_lead))
        .route("/api/v1/leads/:id/assign", post(lead_handler::assign_lead))
        .route("/api/v1/leads/:id/stage", post(lead_handler::update_lead_stage))
        .route("/api/v1/tags", get(tag_handler::list_tags).post(tag_handler::create_tag))
        .route("/api/v1/tags/:id", put(tag_handler::update_tag).delete(tag_handler::delete_tag))
        .route("/api/v1/tag-groups", get(tag_group_handler::list_tag_groups).post(tag_group_handler::create_tag_group))
        .route("/api/v1/tag-groups/:id", put(tag_group_handler::update_tag_group).delete(tag_group_handler::delete_tag_group))
        .route("/api/v1/plan-tag-mappings", get(plan_tag_handler::list_plan_tag_mappings))
        .route("/api/v1/plan-tag-mappings/sync", post(plan_tag_handler::sync_plan_tag_mappings))
        .route("/api/v1/affiliates", get(affiliate_handler::list_affiliates).post(affiliate_handler::create_affiliate))
        .route("/api/v1/affiliates/:id", get(affiliate_handler::get_affiliate).put(affiliate_handler::update_affiliate))
        .route("/api/v1/affiliates/:id/commissions", get(affiliate_handler::get_affiliate_commissions))
        .route("/api/v1/plans", get(plan_handler::list_plans).post(plan_handler::create_plan))
        .route("/api/v1/plans/:id", get(plan_handler::get_plan).put(plan_handler::update_plan))
        .route("/api/v1/dashboard/stats", get(dashboard_handler::get_dashboard_stats))
        .route("/api/v1/dashboard/activity", get(dashboard_handler::get_activity_log))
        .route("/api/v1/webhooks", get(webhook_handler::list_webhooks).post(webhook_handler::create_webhook))
        .route("/api/v1/webhooks/:id", delete(webhook_handler::delete_webhook))
        .route("/api/v1/webhooks/:id/test", post(webhook_handler::test_webhook))
        .route("/api/v1/webhooks/mintbird", post(crate::handlers::mintbird_handler::handle_purchase))
        .route("/api/v1/settings", get(settings_handler::get_settings).put(settings_handler::update_settings))
        .route("/api/v1/target-software", get(routing_handler::list_target_software).post(routing_handler::create_target_software))
        .route("/api/v1/routing-logs", get(routing_handler::list_routing_logs))
        // Portfolio companies
        .route("/api/v1/portfolio-companies", get(portfolio_handler::list_portfolio_companies).post(portfolio_handler::create_portfolio_company))
        .route("/api/v1/portfolio-companies/:id", get(portfolio_handler::get_portfolio_company).put(portfolio_handler::update_portfolio_company).delete(portfolio_handler::delete_portfolio_company))
        // Integration targets
        .route("/api/v1/integration-targets", get(integration_target_handler::list_integration_targets).post(integration_target_handler::create_integration_target))
        .route("/api/v1/integration-targets/:id", put(integration_target_handler::update_integration_target).delete(integration_target_handler::delete_integration_target))
        // API Key management
        .route("/api/v1/api-keys", post(api_key_handler::create_api_key).get(api_key_handler::list_api_keys))
        .route("/api/v1/api-keys/:id", put(api_key_handler::update_api_key).delete(api_key_handler::delete_api_key))

        // Admin endpoints (cross-app portfolio sync + impersonation)
        .route("/api/v1/admin/portfolio-sync", post(crate::handlers::admin_handler::portfolio_sync))
        .route("/api/v1/admin/impersonate", post(crate::handlers::admin_handler::impersonate))
        .route("/api/v1/admin/stop-impersonation", post(crate::handlers::admin_handler::stop_impersonation))
        // Admin plan management
        .route("/api/v1/admin/plans", get(crate::handlers::plan_handler::admin_list_all_plans).post(crate::handlers::plan_handler::admin_create_plan_json))
        .route("/api/v1/admin/plans/assign", post(crate::handlers::plan_handler::admin_assign_plan))
        .route("/api/v1/admin/plans/:id/features", put(crate::handlers::plan_handler::admin_update_plan_features))
        .route("/api/v1/admin/plans/:id", get(crate::handlers::plan_handler::get_plan).put(crate::handlers::plan_handler::update_plan).delete(crate::handlers::plan_handler::delete_plan_admin))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
