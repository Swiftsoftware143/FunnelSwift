-- 048: plans.allowed_template_ids — the template-gating whitelist.
--
-- Three FunnelSwift routes already read/write this column but the column was
-- never created, so each of them returned HTTP 500 ("column
-- allowed_template_ids does not exist"):
--   * GET  /api/v1/admin/plans          (admin_list_all_plans)
--   * GET  /api/v1/admin/plans/:id/templates
--   * PUT  /api/v1/admin/plans/:id/features
--   * PUT  /api/v1/admin/plans/:id/templates
--
-- Semantics already documented in template_gating_handler.rs:
--   NULL  -> all templates allowed (unlocked, the current effective behaviour)
--   []    -> zero templates allowed
--   [...] -> only the listed template ids
-- Nullable and additive: no existing row changes behaviour.

ALTER TABLE plans ADD COLUMN IF NOT EXISTS allowed_template_ids text[];
