-- Migration 057: ONE value space for kinetic_cards.template_type.
--
-- Card: kanban t_747b4dd6. Evidence: /opt/swift/audits/fs-card-template-type-t_747b4dd6/.
--
-- MEASURED on the live `funnelswift` database, 2026-09-25 (a census of the column, not inferred):
--
--     template_type   rows
--     bio-link          12   <- created by a `type`/`card_type` client (HYPHENATED card kind)
--     default           10   <- the column's own DEFAULT / the editor's "Default"
--     business_card      4   <- the card editor's Template Type select (underscore ids)
--     mini_funnel        1   <- same vocabulary
--                        --
--                        27
--
-- So ONE column carried TWO vocabularies: the editor's underscore card types (15 rows) and the
-- hyphenated card KIND that API clients send as `type`/`card_type` ("bio-link", 12 rows).
--
-- DECISION (the load-bearing choice, recorded here AND in the code): `kinetic_cards.template_type`
-- stores the card TYPE, spelled in the editor's underscore ids — the list declared ONCE in
-- www-app/index.html (`var CARD_TYPES=[…]`) and ONCE in src/card_types.rs (`CARD_TYPES`, with a unit
-- test that fails the build if the two copies drift). The hyphenated kind is an ALIAS, not a second
-- value space, because:
--   * it cannot represent three values the column already holds or offers — 'default' (the column's
--     own DEFAULT), 'hero' and 'thank_you' — without inventing ids nobody sends;
--   * every reader already speaks the underscore ids: the editor select, the card badge map, the
--     template-library chips, admin_handler.rs's COALESCE(kc.template_type,'default'), and
--     card_analytics_handler.rs's `card_type` alias;
--   * the plan gate compares them too: kinetic_handler.rs's create_card tests `has_mini_funnels`
--     against the literal "mini_funnel", so a client sending `type:"mini-funnel"` stored a gated
--     card type on a plan that does not grant it (measured: 201 before this card, 402 after).
-- src/card_types.rs now normalises every write (create_card / update_card), so the alias cannot
-- re-enter the column.
--
-- This file canonicalises the rows that already carry an alias. It is a RENAME WITHIN ONE VALUE
-- SPACE, not a mapping between vocabularies: each pair below is the same card type in its two
-- measured spellings, and the app's own text names the alias (`index.html`'s badge map read
-- bio->"Bio Link", business->"Business Card", mini->"Mini Page"; the theme catalogue spells the
-- hero archetype "hero-page"). Only `template_type` is written — no other column, not even
-- updated_at, is touched, so a row-level A/B can prove nothing else moved. Idempotent (0 rows on a
-- re-run), no row inserted or deleted, and any value outside both spaces is left alone and reported.

DO $mig$
DECLARE moved int; total int := 0; leftover text;
BEGIN
    IF to_regclass('public.kinetic_cards') IS NULL THEN
        RAISE NOTICE '057: skipped - kinetic_cards absent on this database';
        RETURN;
    END IF;

    EXECUTE $u$UPDATE kinetic_cards SET template_type = 'bio_link'
                 WHERE lower(btrim(template_type)) IN ('bio-link', 'bio')$u$;
    GET DIAGNOSTICS moved = ROW_COUNT; total := total + moved;
    RAISE NOTICE '057: bio-link/bio -> bio_link: % row(s)', moved;

    EXECUTE $u$UPDATE kinetic_cards SET template_type = 'business_card'
                 WHERE lower(btrim(template_type)) IN ('business-card', 'business')$u$;
    GET DIAGNOSTICS moved = ROW_COUNT; total := total + moved;
    RAISE NOTICE '057: business-card/business -> business_card: % row(s)', moved;

    EXECUTE $u$UPDATE kinetic_cards SET template_type = 'mini_page'
                 WHERE lower(btrim(template_type)) IN ('mini-page', 'mini')$u$;
    GET DIAGNOSTICS moved = ROW_COUNT; total := total + moved;
    RAISE NOTICE '057: mini-page/mini -> mini_page: % row(s)', moved;

    EXECUTE $u$UPDATE kinetic_cards SET template_type = 'mini_funnel'
                 WHERE lower(btrim(template_type)) = 'mini-funnel'$u$;
    GET DIAGNOSTICS moved = ROW_COUNT; total := total + moved;
    RAISE NOTICE '057: mini-funnel -> mini_funnel: % row(s)', moved;

    EXECUTE $u$UPDATE kinetic_cards SET template_type = 'hero'
                 WHERE lower(btrim(template_type)) = 'hero-page'$u$;
    GET DIAGNOSTICS moved = ROW_COUNT; total := total + moved;
    RAISE NOTICE '057: hero-page -> hero: % row(s)', moved;

    -- Values outside the canonical list are NOT rewritten and NOT guessed at: the plan matrix's own
    -- `features.card_types` blurb spells a kind "digital-card" that no code writes or reads, and
    -- inventing a mapping for a spelling the app's own text does not name is the failure mode this
    -- card exists to prevent. The editor appends any unknown stored value to its select, so such a
    -- row stays editable.
    EXECUTE $u$SELECT coalesce(string_agg(DISTINCT template_type, ', ' ORDER BY template_type), '(none)')
                 FROM kinetic_cards
                WHERE lower(btrim(template_type)) NOT IN
                      ('default','bio_link','business_card','mini_page','mini_funnel','hero','thank_you')$u$
      INTO leftover;
    RAISE NOTICE '057: total % row(s) canonicalised; values outside the canonical card types (untouched): %',
                 total, leftover;
END $mig$;
