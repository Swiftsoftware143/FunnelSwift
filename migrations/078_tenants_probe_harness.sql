-- FunnelSwift: mark harness-created tenants at creation (kanban t_fc88ec2a).
--
-- WHY: the signup flow names EVERY workspace `default-<8hex>` / "My Workspace", so a
-- harness-created tenant is byte-indistinguishable from a customer's. That is what made the
-- fleet's 162-root probe tail unattributable and nearly deleted four of the fleet's own accounts.
-- Attribution has to stop being guesswork; this column is the durable half of the answer
-- (/opt/swift/docs/fleet-probe-residue-policy-2026-09-28.md, answer 3c).
--
-- ADDITIVE, NO BACKFILL — on purpose, not an oversight. The column records where a tenant CAME
-- FROM at creation time, and that fact was not captured for the roots that already exist.
-- Inventing a value for them would be a lie in the one column a sweeper will trust. Legacy roots
-- stay classified by the owner-ADDRESS policy (the domain, never the name) in
-- /opt/swift/docs/fleet-probe-residue-allowlist-2026-09-28.txt.
--
-- `tenants` is the ROOT table: it has no parent FK, and ~30 child tables cascade from it
-- (`\d tenants`). The new column is nullable with no default, so the ALTER is catalog-only: no
-- table rewrite, no data rewrite, and not one existing reader or writer of `tenants` changes
-- shape. A NEW migration file because FunnelSwift runs `sqlx::migrate!` at boot, which asserts
-- the checksum of every already-applied file — editing an applied migration HARD-FAILS boot.

ALTER TABLE tenants ADD COLUMN IF NOT EXISTS probe_harness text;

COMMENT ON COLUMN tenants.probe_harness IS
  'Harness marker taken from the X-Swift-Harness request header when the tenant was created (trimmed, lowercased, must match ^[a-z0-9][a-z0-9._-]{2,63}$), else NULL. Header only: never set from a request body or query field, and never set by any other route.';
