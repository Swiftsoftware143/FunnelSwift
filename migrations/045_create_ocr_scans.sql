-- Metering for `max_ocr_scans` (business-card OCR).
--
-- THE BUG THIS FIXES: `plans.max_ocr_scans` was read by src/features.rs
-- (`plan_limit`, which has a `max_ocr_scans` arm) but `get_usage_count` had no
-- matching arm, so it fell through to `_ => 0`. `enforce_feature_limit` then
-- compared `0 >= limit`, which is false for every plan value, so the limit was
-- never enforced and the dashboard usage counter always read 0 — a sold,
-- metered feature shipped completely unmetered.
--
-- One row per business-card scan attempt that reaches the host OCR service
-- (funnelswift-ocr.service on 172.17.0.1:8093). Counted per tenant by
-- `get_usage_count(..., "max_ocr_scans")`.
--
-- Current plan values: Capture Free = 10, Capture Starter = 25,
-- Suite/Agency = NULL (unlimited).

CREATE TABLE IF NOT EXISTS ocr_scans (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_ocr_scans_tenant_id ON ocr_scans (tenant_id);
