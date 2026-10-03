-- FunnelSwift — performance bands: an affiliate's rate rising automatically from real data.
--
-- David, 2026-10-03: *"tiers are unnecessary. But maybe if they have a certain amount of customers I will
-- increase their percentage as an affiliate. Or if they have customers that have multiple apps then they
-- can earn a higher percentage. Is there a way to automate that?"*
--
-- This replaces `affiliate_tiers` (retired in 093), which was the same idea implemented as a MANUAL list
-- nothing read. A band here is a RULE over data the app already records, so there is nothing to keep in
-- sync by hand.
--
-- WHERE THE TWO SIGNALS COME FROM (both already live, nothing new to track):
--   * customers brought   — `leads.created_by` -> `affiliates.user_id`, the permanent tag-bound
--     attribution.
--   * apps per customer   — `tags.source_app` (+ `tags.plan_id`, indexed by `idx_tags_source_app` /
--     `idx_tags_plan`). Each app's plan tag carries its own source_app — the code's own example is
--     "ADASwift — Free" — and `affiliate_tracking_handler::handle_affiliate_upgrade_event` already
--     resolves sibling events by it. So "this customer holds 3 apps" is a DISTINCT COUNT over tags,
--     not new instrumentation.
--
-- WHY A BAND WRITES `affiliates.commission_rate` RATHER THAN BEING READ DURING A CALCULATION:
-- `commission.rs` resolves a rate through six documented sources (personal override -> product group ->
-- product default -> product legacy -> the affiliate's standing rate -> the plan rate). Auto-escalation
-- is implemented by WRITING the standing rate, which is already one of the six. That keeps the money
-- path exactly as simple as it is today: promotion changes a stored number instead of adding a seventh
-- source that every future commission calculation would have to reason about.
--
-- NOT RETROACTIVE, BY CONSTRUCTION: changing `commission_rate` affects commissions computed later.
-- Nothing here rewrites a commission that has already been recorded.
--
-- NO `min_apps_per_customer` DEFAULT OF 0 IS MEANINGFUL: a band requires at least 1 app, so setting
-- `min_apps_per_customer = 2` is what expresses David's "customers that have multiple apps".

CREATE TABLE IF NOT EXISTS affiliate_rate_bands (
    id                     uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    label                  text        NOT NULL,
    min_paying_customers   integer     NOT NULL DEFAULT 0,
    min_apps_per_customer  integer     NOT NULL DEFAULT 1,
    rate                   numeric     NOT NULL,
    sort_order             integer     NOT NULL DEFAULT 0,
    is_active              boolean     NOT NULL DEFAULT true,
    created_at             timestamptz NOT NULL DEFAULT now(),
    updated_at             timestamptz NOT NULL DEFAULT now()
);

-- Seeds are STARTING VALUES for David to edit in the panel, not policy. They are only inserted when the
-- table is empty, so a later edit of his is never overwritten by a re-run of this migration on boot.
INSERT INTO affiliate_rate_bands (label, min_paying_customers, min_apps_per_customer, rate, sort_order)
SELECT * FROM (VALUES
    ('Base',        0,  1, 20.00, 0),
    ('Growing',    10,  1, 25.00, 1),
    ('Established', 25, 1, 28.00, 2),
    ('Portfolio',  10,  2, 30.00, 3)
) AS v(label, min_paying_customers, min_apps_per_customer, rate, sort_order)
WHERE NOT EXISTS (SELECT 1 FROM affiliate_rate_bands);

-- WHY a rate is what it is, in the operator's own words, and which band set it. Without these a rate can
-- only be observed, not explained — and an affiliate who asks "why am I at 25%?" has no answer.
ALTER TABLE affiliates ADD COLUMN IF NOT EXISTS rate_reason   text;
ALTER TABLE affiliates ADD COLUMN IF NOT EXISTS rate_band_id  uuid REFERENCES affiliate_rate_bands(id) ON DELETE SET NULL;
ALTER TABLE affiliates ADD COLUMN IF NOT EXISTS rate_updated_at timestamptz;

-- The recompute reads "affiliates that could move", so index what it filters and orders by.
CREATE INDEX IF NOT EXISTS idx_affiliates_active_rate ON affiliates (is_active, commission_rate);
