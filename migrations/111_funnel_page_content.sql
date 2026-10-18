-- 111 — the funnel's public page carries the customer's own words.
--
-- Before this, `funnels` held a name and a slug and nothing else, so EVERY shared funnel link
-- rendered "This funnel is under construction. Check back soon!" — the console's Funnels screen
-- tells the customer "a funnel is a public page you send people to", and the page it handed them
-- could never say anything or capture anybody.
--
-- Four nullable columns, no defaults: the renderer falls back to the funnel's own name and a plain
-- call to action, so every EXISTING row keeps rendering (better, not broken) without a backfill.
-- `thank_you` is what the visitor sees after submitting, on the page they already loaded.
--
-- Idempotent by construction (`IF NOT EXISTS`) because FunnelSwift's runner re-executes every file
-- it has not recorded; three of the reads below are the ones an operator will need first:
--   SELECT name, headline, subheadline, cta_label, thank_you FROM funnels WHERE slug = $1;
ALTER TABLE funnels ADD COLUMN IF NOT EXISTS headline    text;
ALTER TABLE funnels ADD COLUMN IF NOT EXISTS subheadline text;
ALTER TABLE funnels ADD COLUMN IF NOT EXISTS cta_label   text;
ALTER TABLE funnels ADD COLUMN IF NOT EXISTS thank_you   text;
