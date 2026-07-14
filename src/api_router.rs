use axum::{
    routing::{delete, get, post, put},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tower_http::services::ServeDir;
use axum::routing::any;

use crate::auth::handlers::{login, me, register, change_password, forgot_password, reset_password, update_profile};
use crate::handlers::{
    site_settings_handler,
    public_signup_handler,
    affiliate_handler, api_key_handler, dashboard_handler, lead_handler, linkedin,
    ocr, plan_handler,
    plan_tag_handler, routing_handler, settings_handler, tag_group_handler, tag_handler,
    sync_plan_tag_handler, linkedin_auth_handler, web_to_lead_handler,
    webhook_handler, portfolio_handler, integration_target_handler, affiliate_product_handler, affiliate_tracking_handler, affiliate_portal_handler, cross_app_webhook_handler, affiliate_payout_handler,
    affiliate_lead_handler,
    provider_keys_handler, tenant_handler,
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
        .route("/api/v1/health", get(health))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/signup", post(public_signup_handler::public_signup))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/forgot-password", post(forgot_password))
        .route("/api/v1/auth/reset-password", post(reset_password))
        .route("/api/v1/auth/me", get(me))
        .route("/api/v1/auth/password", put(change_password))
        .route("/api/v1/auth/profile", put(update_profile))
        .route("/api/v1/leads", get(lead_handler::list_leads).post(lead_handler::create_lead))
        .route("/api/v1/leads/export", get(lead_handler::export_leads))
        .route("/api/v1/leads/:id", get(lead_handler::get_lead).put(lead_handler::update_lead).delete(lead_handler::delete_lead))
        .route("/api/v1/leads/:id/assign", post(lead_handler::assign_lead))
        .route("/api/v1/leads/:id/stage", post(lead_handler::update_lead_stage))
        .route("/api/v1/tags", get(tag_handler::list_tags).post(tag_handler::create_tag))
        .route("/api/v1/tags/:id", get(tag_handler::get_tag).put(tag_handler::update_tag).delete(tag_handler::delete_tag))
        .route("/api/v1/tag-groups", get(tag_group_handler::list_tag_groups).post(tag_group_handler::create_tag_group))
        .route("/api/v1/tag-groups/:id", put(tag_group_handler::update_tag_group).delete(tag_group_handler::delete_tag_group))
        .route("/api/v1/plan-tag-mappings", get(plan_tag_handler::list_plan_tag_mappings))
        .route("/api/v1/plan-tag-mappings/sync", post(plan_tag_handler::sync_plan_tag_mappings))
        .route("/api/v1/affiliates", get(affiliate_handler::list_affiliates).post(affiliate_handler::create_affiliate))
        .route("/api/v1/affiliates/:id", get(affiliate_handler::get_affiliate).put(affiliate_handler::update_affiliate).delete(affiliate_handler::delete_affiliate))
        .route("/api/v1/affiliates/:id/commissions", get(affiliate_handler::get_affiliate_commissions))
        .route("/api/v1/affiliate-products", get(affiliate_product_handler::list_affiliate_products).post(affiliate_product_handler::create_affiliate_product))
        .route("/api/v1/affiliate-products/:id", put(affiliate_product_handler::update_affiliate_product).delete(affiliate_product_handler::delete_affiliate_product))
        .route("/api/v1/affiliate-links", get(affiliate_tracking_handler::list_affiliate_links).post(affiliate_tracking_handler::create_affiliate_link))
        .route("/api/v1/affiliate-stats", get(affiliate_tracking_handler::get_affiliate_stats))
        .route("/api/v1/affiliate-conversions", get(affiliate_tracking_handler::list_conversions).post(affiliate_tracking_handler::track_conversion))
        .route("/api/v1/track-click", get(affiliate_tracking_handler::track_click))
        .route("/api/v1/affiliate/signup", post(affiliate_portal_handler::affiliate_signup))
        .route("/api/v1/affiliate/login", post(affiliate_portal_handler::affiliate_login))
        .route("/api/v1/affiliate/dashboard", post(affiliate_portal_handler::affiliate_portal_dashboard))
        .route("/api/v1/affiliate/submit-lead", post(affiliate_lead_handler::submit_affiliate_lead))
        .route("/api/v1/affiliate/leads", post(affiliate_lead_handler::list_affiliate_prospects))
        .route("/api/v1/affiliate/leads-stats", post(affiliate_lead_handler::get_affiliate_leads_stats))
        .route("/api/v1/check-affiliate-email", post(affiliate_lead_handler::check_affiliate_for_email))
        .route("/api/v1/log-lead-movement", post(affiliate_lead_handler::log_lead_movement))
        .route("/api/v1/webhooks/conversion", post(cross_app_webhook_handler::handle_conversion_webhook))
        .route("/api/v1/track/lead", post(cross_app_webhook_handler::track_lead_conversion))
        .route("/api/v1/affiliate-tiers", get(affiliate_payout_handler::list_tiers).post(affiliate_payout_handler::create_tier))
        .route("/api/v1/affiliate-tiers/:id", put(affiliate_payout_handler::update_tier).delete(affiliate_payout_handler::delete_tier))
        .route("/api/v1/affiliates/:id/calculate-tier", post(affiliate_payout_handler::calculate_affiliate_tier))
        .route("/api/v1/affiliate-payouts", get(affiliate_payout_handler::list_payouts).post(affiliate_payout_handler::create_payout))
        .route("/api/v1/affiliate-payouts/:id/pay", post(affiliate_payout_handler::mark_payout_paid))
        .route("/api/v1/affiliates/:id/pending-conversions", get(affiliate_payout_handler::get_affiliate_pending_conversions))
        .route("/api/v1/plans", get(plan_handler::list_plans).post(plan_handler::create_plan))
        .route("/api/v1/plans/:id", get(plan_handler::get_plan).put(plan_handler::update_plan))
        .route("/api/v1/dashboard/stats", get(dashboard_handler::get_dashboard_stats))
        .route("/api/v1/dashboard/activity", get(dashboard_handler::get_activity_log))
        .route("/api/v1/webhooks", get(webhook_handler::list_webhooks).post(webhook_handler::create_webhook))
        .route("/api/v1/webhooks/:id", delete(webhook_handler::delete_webhook))
        .route("/api/v1/webhooks/:id/test", post(webhook_handler::test_webhook))
        .route("/api/v1/settings", get(settings_handler::get_settings).put(settings_handler::update_settings))
        .route("/api/v1/target-software", get(routing_handler::list_target_software).post(routing_handler::create_target_software))
        .route("/api/v1/settings/:key", delete(settings_handler::delete_setting))
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
        // Site settings management (admin only)
        .route("/api/v1/admin/sites", get(site_settings_handler::list_site_settings))
        .route("/api/v1/admin/sites/:slug", get(site_settings_handler::get_site_settings).put(site_settings_handler::update_site_settings))
        // Admin plan management
        .route("/api/v1/admin/plans", get(crate::handlers::plan_handler::admin_list_all_plans).post(crate::handlers::plan_handler::admin_create_plan_json))
        .route("/api/v1/admin/plans/assign", post(crate::handlers::plan_handler::admin_assign_plan))
        .route("/api/v1/admin/plans/:id/features", put(crate::handlers::plan_handler::admin_update_plan_features))
        .route("/api/v1/admin/plans/:id", get(crate::handlers::plan_handler::get_plan).put(crate::handlers::plan_handler::update_plan).delete(crate::handlers::plan_handler::delete_plan_admin))
        // Provider keys management
        .route("/api/v1/provider-keys", get(provider_keys_handler::list_provider_keys).post(provider_keys_handler::upsert_provider_key))
        .route("/api/v1/provider-keys/:provider", delete(provider_keys_handler::delete_provider_key))
        .route("/api/v1/available-providers", get(provider_keys_handler::list_available_providers))
        // Tenant management
        .route("/api/v1/tenants", get(tenant_handler::list_tenants).post(tenant_handler::create_tenant))
        .route("/api/v1/tenants/:id", get(tenant_handler::get_tenant).put(tenant_handler::update_tenant).delete(tenant_handler::delete_tenant))
        .route("/api/v1/tenants/:id/credits", get(tenant_handler::get_tenant_credits).post(tenant_handler::assign_credits))
        .route("/api/v1/tenants/:id/plan", post(tenant_handler::assign_plan))
        .route("/api/v1/internal/sync-plan-tag", post(sync_plan_tag_handler::sync_plan_tag))
        // OCR - business card parsing
        .route("/api/v1/ocr/parse-card", post(ocr::handle_parse_card))
        // LinkedIn - profile lookup
        .route("/api/v1/leads/linkedin-lookup", post(linkedin::handle_linkedin_lookup))
        .route("/api/v1/linkedin/auth", post(linkedin_auth_handler::store_linkedin_auth))
        .route("/api/v1/linkedin/auth/status", get(linkedin_auth_handler::get_linkedin_auth_status))
        .route("/api/v1/linkedin/auth", delete(linkedin_auth_handler::delete_linkedin_auth))
        .route("/api/v1/linkedin/cookies/:user_id", get(linkedin_auth_handler::get_linkedin_cookies_for_user))
        .route("/api/v1/web-to-lead/configs", get(web_to_lead_handler::list_web_to_lead_configs).post(web_to_lead_handler::create_web_to_lead_config))
        .route("/api/v1/web-to-lead/configs/:id", put(web_to_lead_handler::update_web_to_lead_config).delete(web_to_lead_handler::delete_web_to_lead_config))
        .route("/api/v1/web-to-lead/configs/:id/embed", get(web_to_lead_handler::get_web_to_lead_embed))
        .route("/api/v1/web-to-lead", post(web_to_lead_handler::handle_web_to_lead))
        .nest("/downloads", axum::Router::new().fallback_service(ServeDir::new("/tmp")))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
