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
    clients_handler, workflows_handler, deals_handler, campaigns_handler, tickets_handler,
    email_templates_handler, reviews_handler, surfaces_handler, categories_handler,
    reports_handler, knowledge_base_handler, import_logs_handler, export_templates_handler,
    call_logs_handler, calendar_events_handler,
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
        .route("/", get(|| async { axum::Json(serde_json::json!({"status": "ok", "service": "funnelswift"})) }))
        .route("/api/health", get(health))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/forgot-password", post(forgot_password))
        .route("/api/auth/reset-password", post(reset_password))
        .route("/api/auth/me", get(me))
        .route("/api/auth/password", put(change_password))
        .route("/api/leads", get(lead_handler::list_leads).post(lead_handler::create_lead))
        .route("/api/leads/export", get(lead_handler::export_leads))
        .route("/api/leads/:id", get(lead_handler::get_lead).put(lead_handler::update_lead).delete(lead_handler::delete_lead))
        .route("/api/leads/:id/assign", post(lead_handler::assign_lead))
        .route("/api/leads/:id/stage", post(lead_handler::update_lead_stage))
        .route("/api/tags", get(tag_handler::list_tags).post(tag_handler::create_tag))
        .route("/api/tags/:id", put(tag_handler::update_tag).delete(tag_handler::delete_tag))
        .route("/api/tag-groups", get(tag_group_handler::list_tag_groups).post(tag_group_handler::create_tag_group))
        .route("/api/tag-groups/:id", put(tag_group_handler::update_tag_group).delete(tag_group_handler::delete_tag_group))
        .route("/api/plan-tag-mappings", get(plan_tag_handler::list_plan_tag_mappings))
        .route("/api/plan-tag-mappings/sync", post(plan_tag_handler::sync_plan_tag_mappings))
        .route("/api/affiliates", get(affiliate_handler::list_affiliates).post(affiliate_handler::create_affiliate))
        .route("/api/affiliates/:id", get(affiliate_handler::get_affiliate).put(affiliate_handler::update_affiliate))
        .route("/api/affiliates/:id/commissions", get(affiliate_handler::get_affiliate_commissions))
        .route("/api/plans", get(plan_handler::list_plans).post(plan_handler::create_plan))
        .route("/api/plans/:id", get(plan_handler::get_plan).put(plan_handler::update_plan))
        .route("/api/dashboard/stats", get(dashboard_handler::get_dashboard_stats))
        .route("/api/dashboard/activity", get(dashboard_handler::get_activity_log))
        .route("/api/webhooks", get(webhook_handler::list_webhooks).post(webhook_handler::create_webhook))
        .route("/api/webhooks/:id", delete(webhook_handler::delete_webhook))
        .route("/api/webhooks/:id/test", post(webhook_handler::test_webhook))
        .route("/api/settings", get(settings_handler::get_settings).put(settings_handler::update_settings))
        .route("/api/target-software", get(routing_handler::list_target_software).post(routing_handler::create_target_software))
        .route("/api/routing-logs", get(routing_handler::list_routing_logs))
        // Portfolio companies
        .route("/api/portfolio-companies", get(portfolio_handler::list_portfolio_companies).post(portfolio_handler::create_portfolio_company))
        .route("/api/portfolio-companies/:id", get(portfolio_handler::get_portfolio_company).put(portfolio_handler::update_portfolio_company).delete(portfolio_handler::delete_portfolio_company))
        // Integration targets
        .route("/api/integration-targets", get(integration_target_handler::list_integration_targets).post(integration_target_handler::create_integration_target))
        .route("/api/integration-targets/:id", put(integration_target_handler::update_integration_target).delete(integration_target_handler::delete_integration_target))
        // API Key management
        .route("/api/api-keys", post(api_key_handler::create_api_key).get(api_key_handler::list_api_keys))
        .route("/api/api-keys/:id", put(api_key_handler::update_api_key).delete(api_key_handler::delete_api_key))

        // Admin endpoints (cross-app portfolio sync + impersonation)
        .route("/api/admin/portfolio-sync", post(crate::handlers::admin_handler::portfolio_sync))
        .route("/api/admin/impersonate", post(crate::handlers::admin_handler::impersonate))
        .route("/api/admin/stop-impersonation", post(crate::handlers::admin_handler::stop_impersonation))
        // Admin plan management
        .route("/api/admin/plans", get(crate::handlers::plan_handler::admin_list_all_plans).post(crate::handlers::plan_handler::admin_create_plan_json))
        .route("/api/admin/plans/assign", post(crate::handlers::plan_handler::admin_assign_plan))
        .route("/api/admin/plans/:id/features", put(crate::handlers::plan_handler::admin_update_plan_features))
        .route("/api/admin/plans/:id", get(crate::handlers::plan_handler::get_plan).put(crate::handlers::plan_handler::update_plan).delete(crate::handlers::plan_handler::delete_plan_admin))
        
        // Clients
        .route("/api/clients", get(clients_handler::list).post(clients_handler::create))
        .route("/api/clients/:id", get(clients_handler::get).put(clients_handler::update).delete(clients_handler::delete))
        // Workflows
        .route("/api/workflows", get(workflows_handler::list).post(workflows_handler::create))
        .route("/api/workflows/:id", get(workflows_handler::get).put(workflows_handler::update).delete(workflows_handler::delete))
        // Deals
        .route("/api/deals", get(deals_handler::list).post(deals_handler::create))
        .route("/api/deals/:id", get(deals_handler::get).put(deals_handler::update).delete(deals_handler::delete))
        // Campaigns
        .route("/api/campaigns", get(campaigns_handler::list).post(campaigns_handler::create))
        .route("/api/campaigns/:id", get(campaigns_handler::get).put(campaigns_handler::update).delete(campaigns_handler::delete))
        // Tickets
        .route("/api/tickets", get(tickets_handler::list).post(tickets_handler::create))
        .route("/api/tickets/:id", get(tickets_handler::get).put(tickets_handler::update).delete(tickets_handler::delete))
        // Email Templates
        .route("/api/email-templates", get(email_templates_handler::list).post(email_templates_handler::create))
        .route("/api/email-templates/:id", get(email_templates_handler::get).put(email_templates_handler::update).delete(email_templates_handler::delete))
        // Reviews
        .route("/api/reviews", get(reviews_handler::list).post(reviews_handler::create))
        .route("/api/reviews/:id", get(reviews_handler::get).put(reviews_handler::update).delete(reviews_handler::delete))
        // Surfaces
        .route("/api/surfaces", get(surfaces_handler::list).post(surfaces_handler::create))
        .route("/api/surfaces/:id", get(surfaces_handler::get).put(surfaces_handler::update).delete(surfaces_handler::delete))
        // Categories
        .route("/api/categories", get(categories_handler::list).post(categories_handler::create))
        .route("/api/categories/:id", get(categories_handler::get).put(categories_handler::update).delete(categories_handler::delete))
        // Reports
        .route("/api/reports", get(reports_handler::list).post(reports_handler::create))
        .route("/api/reports/:id", get(reports_handler::get).put(reports_handler::update).delete(reports_handler::delete))
        // Knowledge Base
        .route("/api/knowledge-base", get(knowledge_base_handler::list).post(knowledge_base_handler::create))
        .route("/api/knowledge-base/:id", get(knowledge_base_handler::get).put(knowledge_base_handler::update).delete(knowledge_base_handler::delete))
        // Import Logs
        .route("/api/import-logs", get(import_logs_handler::list).post(import_logs_handler::create))
        .route("/api/import-logs/:id", get(import_logs_handler::get).put(import_logs_handler::update).delete(import_logs_handler::delete))
        // Export Templates
        .route("/api/export-templates", get(export_templates_handler::list).post(export_templates_handler::create))
        .route("/api/export-templates/:id", get(export_templates_handler::get).put(export_templates_handler::update).delete(export_templates_handler::delete))
        // Call Logs
        .route("/api/call-logs", get(call_logs_handler::list).post(call_logs_handler::create))
        .route("/api/call-logs/:id", get(call_logs_handler::get).put(call_logs_handler::update).delete(call_logs_handler::delete))
        // Calendar Events
        .route("/api/calendar-events", get(calendar_events_handler::list).post(calendar_events_handler::create))
        .route("/api/calendar-events/:id", get(calendar_events_handler::get).put(calendar_events_handler::update).delete(calendar_events_handler::delete))

        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
