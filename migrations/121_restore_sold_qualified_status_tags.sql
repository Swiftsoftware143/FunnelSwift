-- Migration 121: RESTORE the two lifecycle system tags (Sold, Qualified) and the
-- "Sold removes Qualified" rule that `tag_logic` reads by compile-time id.
--
-- MEASURED DEFECT (live `funnelswift`, 2026-10-31 14:42 UTC, container log):
--
--   WARN funnelswift::tag_logic: Sold system tag (id=3b008e4a-dbc8-5558-8762-2e1787ec7c2c)
--        not found - skipping auto-apply for tenant b123b1e7-… after to_plan_slug=kinetic-pro
--
-- and the state behind it, read on the same database:
--   SELECT count(*) FROM tags WHERE id IN (Sold, Qualified)  -> 0
--   SELECT count(*) FROM tag_groups                          -> 0
--   SELECT count(*) FROM tag_rules                           -> 0
--
-- so on live, TODAY: a customer who buys a plan gets no lead marked Sold, the "Sold removes
-- Qualified" rule cannot fire, and the admin System Tags screen has no lifecycle pair to show.
-- `apply_sold_to_tenant_leads` asks for the Sold row by the id constant in src/tag_logic.rs:83-86
-- and returns 0 when it is absent — a silent skip, not an error, which is why it survived.
--
-- This is the same shape migration 115 measured on 2026-10-22 for the seven free-plan tags
-- (`count(*) FROM tags` = 3: not one system tag existed, tag-driven signup dead). 060 had already
-- re-seeded the whole taxonomy on 2026-09-26 and recorded `success = true`; the rows were gone
-- afterwards, and nothing in the app re-creates them at boot. The repair path is a migration, so
-- this file is that repair — and migration 121's code half (`tag_logic::is_platform_reserved`,
-- enforced in `DELETE|PUT /api/v1/tags/:id`) is what stops it happening a third time.
--
-- Scope is deliberately narrow: the STATUS group, Sold + Qualified, and the rule that keeps them
-- exclusive. The 22 shared vocabulary tags of 000001 are NOT re-seeded here — an operator may have
-- retired those on purpose, and nothing in the crate reads them.
--
-- Idempotent: every statement is guarded (by id, by name, or by the rule's name), so a re-run on a
-- repaired database writes nothing. No BEGIN/COMMIT — sqlx wraps each migration in its own
-- transaction.

DO $mig$
DECLARE
    sys_tenant   uuid := '00000000-0000-0000-0000-000000000001';
    status_group uuid := 'a0000000-0000-0000-0000-000000000002';
    sold_id      uuid := '3b008e4a-dbc8-5558-8762-2e1787ec7c2c';
    qualified_id uuid := '15698a9a-67fe-5bf1-9aac-1dcd7a1ccd9e';
    tags_before  int;
    tags_after   int;
BEGIN
    IF to_regclass('public.tags') IS NULL OR to_regclass('public.tenants') IS NULL THEN
        RAISE NOTICE '121: skipped - tags/tenants absent on this database';
        RETURN;
    END IF;

    SELECT count(*) INTO tags_before FROM tags;

    -- 1. The System tenant is the documented home of the shared vocabulary (043/060/115 headers).
    INSERT INTO tenants (id, name, slug)
    VALUES (sys_tenant, 'System', 'system')
    ON CONFLICT DO NOTHING;

    -- 2. The Status group those two tags belong to (000001_initial.sql:253-260, verbatim id).
    IF to_regclass('public.tag_groups') IS NOT NULL THEN
        INSERT INTO tag_groups (id, tenant_id, name, is_collapsible, sort_order)
        VALUES (status_group, sys_tenant, 'Status', true, 2)
        ON CONFLICT (id) DO NOTHING;
    END IF;

    -- 3. Sold + Qualified, under their ORIGINAL deterministic ids — the ids are constants in
    --    src/tag_logic.rs, so a row under any other id would not help. A guard on the NAME keeps a
    --    re-run from creating a second "Sold" beside one an operator re-created by hand.
    INSERT INTO tags (id, tenant_id, group_id, name, color, is_system)
    SELECT sold_id, sys_tenant, status_group, 'Sold', '#FF9800', true
    WHERE NOT EXISTS (SELECT 1 FROM tags WHERE id = sold_id)
      AND NOT EXISTS (SELECT 1 FROM tags WHERE lower(name) = 'sold' AND is_system = true);

    INSERT INTO tags (id, tenant_id, group_id, name, color, is_system)
    SELECT qualified_id, sys_tenant, status_group, 'Qualified', '#4CAF50', true
    WHERE NOT EXISTS (SELECT 1 FROM tags WHERE id = qualified_id)
      AND NOT EXISTS (SELECT 1 FROM tags WHERE lower(name) = 'qualified' AND is_system = true);

    -- 4. "Sold removes Qualified" (0014_sold_qualified_tags.sql:15-27, 060 step 6). The two ids are
    --    SELECTed back by NAME, so the rule follows whichever row actually exists if an operator had
    --    re-created one of the tags under a different id.
    IF to_regclass('public.tag_rules') IS NOT NULL THEN
        INSERT INTO tag_rules (id, tenant_id, name, description, trigger_tag_id, action_type,
                               action_tag_id, target_app, is_active)
        SELECT 'dcdd1042-9a01-5797-8e14-d2653825c74c', sys_tenant, 'Sold removes Qualified',
               'When a lead gets the Sold tag, auto-remove the Qualified tag',
               s.id, 'remove_tag', q.id, 'funnelswift', true
          FROM (SELECT id FROM tags WHERE lower(name) = 'sold' AND is_system = true LIMIT 1) s,
               (SELECT id FROM tags WHERE lower(name) = 'qualified' AND is_system = true LIMIT 1) q
        WHERE NOT EXISTS (SELECT 1 FROM tag_rules WHERE name = 'Sold removes Qualified');
    END IF;

    SELECT count(*) INTO tags_after FROM tags;
    RAISE NOTICE '121: tags % -> % (Sold/Qualified + Status group + the Sold-removes-Qualified rule restored)',
        tags_before, tags_after;
END
$mig$;
