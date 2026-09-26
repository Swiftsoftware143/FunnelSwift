-- 063 — RETIRE `plans.features.card_types` (kanban t_cbd500ca)
--
-- DECISION (the deliverable): the plan matrix's card-kind list is **RETIRED**, not promoted to an
-- enforced per-plan gate. Recorded here so the next reader does not re-open the card.
--
-- Why RETIRE — measured, not preferred:
--   1. It is a THIRD spelling of the card kinds. Every plan that carried it held
--      ["bio-link","digital-card","mini-page"], while the app's ONE card-type value space
--      (`src/card_types.rs::CARD_TYPES`) is default|bio_link|business_card|mini_page|mini_funnel|
--      hero|thank_you. `digital-card` exists nowhere else in the product (canonical("digital-card")
--      == None, pinned by card_types::tests), and `bio-link`/`mini-page` are ALIASES that
--      `card_types::canonical` normalises to bio_link/mini_page — so a gate comparing the stored
--      blurb verbatim against CARD_TYPES would match neither of them.
--   2. NO code reads it: `grep -rn "card_types" src/ www-app/ www-admin/ migrations/ docs/ scripts/`
--      returns only this card's own doc comments (kanban t_747b4dd6) — no route, no gate, no UI, no
--      template, no test. A sweep of every jsonb/json column in this database for the key name
--      (`information_schema.columns` -> `WHERE <col> ? 'card_types'`) hits `plans.features` only.
--   3. The list is WRONG about its own plans, both ways, so it cannot be promoted as-is:
--        * UNDER-inclusive — suite and agency carry `mini_funnels: true`, i.e. their own enforced
--          gate (`features::enforce_feature_flag(..., "has_mini_funnels")`, read by
--          kinetic_handler.rs::create_card) says the plan MAY create a mini_funnel, while the list
--          omits it. Promoting the blurb would take a card type away from paying plans.
--        * OVER-inclusive — it offers `digital-card`, which this service cannot produce at all.
--   4. Which card kinds a plan sells is ALREADY enforced, as a BOOLEAN flag: the `has_mini_funnels`
--      column plus the `mini_funnels` jsonb override (features.rs::flag_jsonb_key). That vocabulary
--      has no list shape at all; promoting `card_types` would invent a new enforcement primitive for
--      a policy the boolean already owns — two sources of truth in two vocabularies, which is
--      exactly the drift t_747b4dd6 just closed. Which plan should sell which archetype is a product
--      decision (architecture lane), not something a dead jsonb key becomes by being "wired up".
--
-- So the key is removed from every plan that carried it. Rows are NOT otherwise rewritten — not even
-- `updated_at` — so a row-level A/B can prove that only `features` moved, and only on those rows.
--
-- Sticky by construction: `plan_handler.rs::reject_retired_feature_keys` now REFUSES a `features`
-- object that carries `card_types` (400, with the canonical ids in the message) on all four plan
-- write paths, so no future UI or gate can re-open the drift by writing the key back.
--
-- Idempotent (re-run = 0 rows) and guarded (a database whose `plans` table is absent skips with a
-- NOTICE). NOTICEs, never RAISE EXCEPTION, for the post-state: a hard assert would refuse the boot
-- on an empty database. No transaction control, no `;` inside prose comments.
DO $mig$
DECLARE
    carried  int;
    removed  int;
    still    int;
    odd      int;
    vals     text;
    others   text;
BEGIN
    IF to_regclass('public.plans') IS NULL THEN
        RAISE NOTICE '063: skipped - plans absent on this database';
        RETURN;
    END IF;

    EXECUTE $c$SELECT count(*) FROM plans WHERE features ? 'card_types'$c$ INTO carried;

    -- What is about to be thrown away, recorded (so the dump is not the only copy of the blurb).
    -- (jsonb_array_elements_text is evaluated in the projection of a subquery whose WHERE already
    -- filtered by jsonb_typeof, so a bare scalar under the key cannot blow the boot up.)
    EXECUTE $c$SELECT coalesce(string_agg(DISTINCT v, ', '), '(none)')
                 FROM (SELECT jsonb_array_elements_text(p.features->'card_types') AS v
                         FROM plans p
                        WHERE p.features ? 'card_types'
                          AND jsonb_typeof(p.features->'card_types') = 'array') s$c$ INTO vals;

    -- A non-array value (a bare string, say) is removed too, but it is NOT silently "the same blurb".
    EXECUTE $c$SELECT count(*) FROM plans
                 WHERE features ? 'card_types'
                   AND jsonb_typeof(features->'card_types') <> 'array'$c$ INTO odd;

    RAISE NOTICE '063: plans carrying features.card_types: % (values: %; non-array: %)',
                 carried, vals, odd;

    EXECUTE $u$UPDATE plans SET features = features - 'card_types'
                 WHERE features ? 'card_types'$u$;
    GET DIAGNOSTICS removed = ROW_COUNT;
    RAISE NOTICE '063: features.card_types removed from % plan(s)', removed;

    -- Post-state census: the key must be ABSENT, and no sibling card-kind list may have appeared.
    EXECUTE $c$SELECT count(*) FROM plans WHERE features ? 'card_types'$c$ INTO still;
    EXECUTE $c$SELECT coalesce(string_agg(DISTINCT k, ', '), '(none)')
                 FROM (SELECT jsonb_object_keys(p.features) AS k
                         FROM plans p
                        WHERE jsonb_typeof(p.features) = 'object') s
                WHERE k LIKE 'card_type%'$c$ INTO others;

    RAISE NOTICE '063: after: plans carrying features.card_types: %; other card_type* keys: %',
                 still, others;
END $mig$;
