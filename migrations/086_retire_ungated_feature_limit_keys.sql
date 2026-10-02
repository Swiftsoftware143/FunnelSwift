-- 086_retire_ungated_feature_limit_keys.sql — RETIRE the 9 `feature_limits` keys NO gate reads.
-- kanban t_090f3e00 (follow-up of t_35acff73, which registered every GATE key and made the grants
-- panel-manageable, but deliberately did not change any key's enforcement).
--
-- WHY THIS FILE EXISTS
--   MEASURED 2026-10-02 on the live database: `feature_limits` (87 rows) carried 39 rows for 9 keys
--   that NO gate in this crate reads. `src/feature_registry.rs` lists the 16 numeric keys the gates
--   are actually called with; not one of these 9 is among them, and a scan of `src/` for each
--   literal finds no call site, no SQL parameter and no resolver arm. A number the operator authored
--   as pricing that no code consumes is worse than a missing feature — it is a knob that looks
--   enforcing. Each of the 9 is either
--     (a) a SECOND NAME for a capability another registry key already enforces, or
--     (b) a quantity this app never measures, or measures but has no route that writes it.
--
--   key                     | verdict | survivor that DOES enforce the same capability      | why
--   ------------------------+---------+-----------------------------------------------------+----------------------------------------
--   kinetic_themes          | retire  | `premium_themes` (boolean, plans.features jsonb)     | premium theme access is a toggle, read by
--                           |         | enforce_theme_access/enforce_template_access         | enforce_theme_access/enforce_template_access.
--                           |         | on POST /api/v1/kinetic/cards                        | The numeric rows contradict it: capture-starter
--                           |         |                                                      | (6) / capture-free (3) carry NO `premium_themes`
--                           |         |                                                      | key, so every premium theme is refused on them.
--   max_api_calls           | retire  | (none — no meter exists)                             | no request counter anywhere: no call/usage
--                           |         |                                                      | table, no per-period accounting. Carded.
--   max_plans               | retire  | (none — no such quantity)                            | `plans` is the global price list (6 rows, no
--                           |         |                                                      | tenant_id); a tenant is not capped on plans. Both
--                           |         |                                                      | authored values are -1, so a wire would be a no-op.
--   max_portfolio_companies | retire  | `max_portfolios`                                     | same COUNT(portfolio_companies) as the gate's key
--                           |         | POST /api/v1/portfolio-companies                     | (`max_portfolios`), added in the SAME 2026-07-06
--                           |         |                                                      | insert wave as this name.
--   max_routing_rules       | retire  | `max_routing_targets` / `max_integrations`           | the phrase "routing rule" exists nowhere in src/ or
--                           |         | POST /api/v1/target-software,                        | the served consoles; the routing entity is
--                           |         | POST /api/v1/integration-targets                     | `target_software`, already counted by two keys.
--   max_settings            | retire  | (none — no such entitlement)                         | the only candidate quantity is COUNT(tenant_settings),
--                           |         |                                                      | an internal key/value store, not a plan entitlement.
--                           |         |                                                      | Both authored values are -1 => a wire would be a no-op.
--   max_target_software     | retire  | `max_routing_targets` / `max_integrations`           | a THIRD name for COUNT(target_software); two
--                           |         |                                                      | registry keys already answer that question.
--   storage_mb              | retire  | (none — no size accounting)                          | nothing in the crate measures bytes or rows stored
--                           |         |                                                      | (no octet_length / pg_total_relation_size / SUM(size)).
--                           |         |                                                      | Carded.
--   team_members            | retire  | `max_team_members` (registry key, column-backed)     | the resolver arm (COUNT(users ... is_active)) and the
--                           |         |                                                      | `team_members`->`max_team_members` column alias already
--                           |         |                                                      | exist; this is the second spelling of one cap. NO route
--                           |         |                                                      | adds a 2nd member to an EXISTING tenant, so the cap is
--                           |         |                                                      | inert either way. Carded.
--
-- DECISION: all 9 are RETIRED, and retired means DELETED — a deactivated row is NOT enough, because
--   `resolved_limit` reads any `feature_limits` row for the key the moment a gate is called with it,
--   so a leftover row is a loaded gun, not a memento. ONE vocabulary per capability.
--
--   The list is preserved in code as `feature_registry::RETIRED_KEYS` (key + reason):
--     * GET /api/v1/admin/plans/registry publishes it as `retired_keys`, so the operator's console
--       can say "these keys are retired and enforce nothing" instead of leaving a blank;
--     * the drift test in that file refuses to let a retired key be re-registered as a gate key.
--
-- AUTHORED NUMBERS — the values that were in `feature_limits` immediately before this file ran.
--   They survive HERE, in the admin guide's "Retired plan keys" section, and in
--   /opt/swift/audits/t_090f3e00/ (00-before-*.txt). They are deliberately NOT left as live rows,
--   so nothing downstream can mistake them for an enforced cap. If a capability below is ever
--   actually implemented, the number is a starting point for the owner — not a promise made now.
--
--     slug            | kinetic_themes | max_api_calls | max_plans | max_portfolio_companies | max_routing_rules | max_settings | max_target_software | storage_mb | team_members
--     ----------------+----------------+---------------+-----------+-------------------------+-------------------+--------------+---------------------+------------+-------------
--     agency          |       -1       |      -1       |    -1     |           -1            |        -1         |     -1       |         -1          |     -1     |      -1
--     capture-free    |        3       |     1000      |     —     |            —            |         —         |      —       |          —          |     50     |       1
--     capture-starter |        6       |     1000      |     —     |            —            |         —         |      —       |          —          |     50     |       1
--     kinetic-free    |        —       |    10000      |     —     |            —            |         —         |      —       |          —          |    500     |       3
--     kinetic-pro     |       -1       |      -1       |    -1     |           100           |        100        |     -1       |         50          |     -1     |      -1
--     (10 further rows, 2026-06-27/07-26 wave, carried plan_ids of plans that no longer exist —
--      deleted by this file too: max_api_calls 1000 x3, storage_mb 50 x3, team_members 1 x3,
--      kinetic_themes 3 x1.)
--
--   MEASURED EFFECT: feature_limits 87 -> 48; 39 rows retired (29 on live plans + 10 orphaned);
--   the 16 registry limit keys and the whole `plans` table are byte-identical (fingerprints in the
--   audit dir). No entitlement any gate can read moves.
--
-- IDEMPOTENT: delete-by-key. A second application removes 0 rows and re-asserts the same invariant.
--   The boot runner applies this file as ONE transaction, so it lands whole or not at all.
DO $retire$
DECLARE
    retired text[] := ARRAY[
        'kinetic_themes', 'max_api_calls', 'max_plans', 'max_portfolio_companies',
        'max_routing_rules', 'max_settings', 'max_target_software', 'storage_mb', 'team_members'
    ];
    n_before  int;
    n_after   int;
    n_removed int;
BEGIN
    SELECT count(*) INTO n_before FROM feature_limits;

    DELETE FROM feature_limits WHERE feature_key = ANY (retired);
    GET DIAGNOSTICS n_removed = ROW_COUNT;

    -- A retire that leaves a row behind is the exact failure this file exists to prevent: the key
    -- would still resolve the moment any gate is called with it.
    IF EXISTS (SELECT 1 FROM feature_limits WHERE feature_key = ANY (retired)) THEN
        RAISE EXCEPTION 'retire: an ungated feature_limits key survived the migration';
    END IF;

    SELECT count(*) INTO n_after FROM feature_limits;
    RAISE NOTICE 'retire: feature_limits % -> %; % row(s) for 9 ungated key(s) retired',
        n_before, n_after, n_removed;
END
$retire$;
