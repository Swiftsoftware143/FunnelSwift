// `adaswift_provision` (stub handlers for the ADASwift push family) was DELETED here on
// 2026-09-25 — kanban t_0a6a93f1. Every one of its four functions either fabricated a success
// reply while writing nothing, or only wrote a log line. See api_router.rs for the per-leg reason.
pub mod admin_handler;
pub mod affiliate_commission_handler;
pub mod affiliate_handler;
pub mod affiliate_lead_handler;
pub mod affiliate_onboarding_handler;
pub mod affiliate_payout_handler;
pub mod affiliate_portal_handler;
pub mod affiliate_product_handler;
pub mod affiliate_referral_handler;
pub mod affiliate_tracking_handler;
pub mod api_key_handler;
pub mod bulk_handler;
pub mod campaigns_handler;
pub mod card_analytics_handler;
pub mod checkout_handler;
pub mod coreswift_integration_handler;
pub mod coreswift_push;
pub mod cross_app_webhook_handler;
pub mod dashboard_handler;
pub mod email_template_handler;
pub mod funnel_handler;
pub mod incentiveswift_handler;
pub mod insight_handler;
// RETIRED (kanban t_0aaf0bc5): `pub mod integration_target_handler;` — a second route family
// over the same `target_software` table as routing_handler, 0 rows, no dispatcher. See migration 088.
pub mod kinetic_handler;
pub mod lead_handler;
pub mod linkedin;
pub mod linkedin_auth_handler;
pub mod ocr;
pub mod plan_handler;
// RETIRED (kanban t_dc418458): `pub mod plan_tag_handler;` — the only reader of
// `plan_tag_mappings` (0 rows, no consumer) was its own list/sync CRUD. Both routes, the handler
// and the table were retired together; see the RETIRED note in `src/api_router.rs`.
pub mod portfolio_handler;
pub mod portfolio_sync_handler;
pub mod product_category_handler;
pub mod provider_keys_handler;
pub mod public_signup_handler;
pub mod qr_handler;
// RETIRED (kanban t_0aaf0bc5): `pub mod routing_handler;` — CRUD for the `target_software` table
// whose only live field (`api_key`) for the IncentiveSwift hand-off moved to `provider_keys`.
pub mod seo_handler;
pub mod settings_handler;
pub mod site_handler;
pub mod site_settings_handler;
pub mod tag_group_handler;
pub mod tag_handler;
pub mod tag_rule_handler;
pub mod template_gating_handler;
pub mod tenant_handler;
pub mod theme_endpoint;
pub mod web_to_lead_handler;
pub mod webhook_handler;
pub mod workflowswift_push;
