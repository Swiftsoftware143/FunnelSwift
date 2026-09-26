-- Migration 060: put the System-tenant tag taxonomy back, and link it to the products.
--
-- WHY THIS FILE EXISTS (kanban t_c06d643c, measured on the live database 2026-09-26)
--
--   The tag vocabulary this app routes affiliate money on was declared by three migrations and ALL
--   of it was destroyed by one CASCADE. The rows lived under the System tenant
--   00000000-0000-0000-0000-000000000001, and both tags.tenant_id and affiliate_products.tenant_id
--   are FOREIGN KEY ... ON DELETE CASCADE, so deleting that tenant took:
--     000001_initial.sql:253-286  6 tag_groups + 22 shared system tags (Referral, Website, Active,
--                                 Won, Lost, Hot, Cold, New Lead, ...)
--     043_seed_free_affiliate_products.sql:13-39  the 7 per-app Free tags
--     0014_sold_qualified_tags.sql:5-27           the Sold and Qualified tags + the
--                                 "Sold removes Qualified" rule
--
--   Measured post-state before this migration: tags 5 rows and is_system = 0, tag_groups 1 row
--   (a tenant probe group), tag_rules 0 rows, and every affiliate_products row with
--   system_tag_id IS NULL. That last one is what makes the money path structurally dead: the
--   reader tag_logic::attribute_affiliate_on_tags (src/tag_logic.rs:319) resolves a product with
--
--       SELECT id FROM affiliate_products WHERE system_tag_id = ANY($1) AND is_active = true
--
--   so with no tag linked to no product it credits nothing for any tag a lead acquires. Sold is a
--   second casualty of the same deletion - tag_logic::SOLD_TAG_NAME is not resolvable either.
--
--   All three migrations are recorded in _sqlx_migrations with success = t and checksums matching
--   their files byte for byte, so none of them will ever re-run and a new migration is the only
--   route back. That is the same shape as 055, which restored the System tenant itself and the five
--   per-app Free products.
--
-- THE DECISION (recorded here and at the reader in src/tag_logic.rs)
--
--   The tag vocabulary IS product-owned platform seed data, not tenant data and not dead weight:
--   the Free tag of each app is the routing signal for that app's free tier, and Sold/Qualified are
--   the lifecycle tags the plan-upgrade path applies (tag_logic.rs:88-121). So this migration
--   RESTORES the declared vocabulary rather than deleting the reader, and links each product to its
--   tag. Names and colours are 043's and 0014's verbatim - not re-invented - so 043's exact strings
--   and 0014's deterministic tag ids (which the Rust constants SOLD_TAG_ID / QUALIFIED_TAG_ID
--   reference) survive the round trip.
--
--   Idempotent, guarded, NOTICE-only post-state: this runner logs a migration error and starts
--   anyway (src/db.rs:36), so a hard assert could never be the evidence and a silent no-op has to be
--   visible in the boot log instead.
--
-- NO SEMICOLONS IN THIS HEADER, deliberately (the deploy staging path splits on the statement
-- separator, as 050 and 0059 record).

DO $mig$
DECLARE
    sys_tenant uuid := '00000000-0000-0000-0000-000000000001';
    status_group uuid := 'a0000000-0000-0000-0000-000000000002';
    groups_restored int := 0;
    shared_restored int := 0;
    free_restored int := 0;
    lifecycle_restored int := 0;
    rule_restored int := 0;
    links_set int := 0;
    sys_tags int := 0;
    linked int := 0;
    census text;
    unlinked text;
    dups text;
    missing text;
BEGIN
    IF to_regclass('public.tags') IS NULL
       OR to_regclass('public.tenants') IS NULL
       OR to_regclass('public.tag_groups') IS NULL THEN
        RAISE NOTICE '060: skipped - tags/tenants/tag_groups absent on this database';
        RETURN;
    END IF;

    -- 1. The home of the vocabulary. 055 already restores it for its own purposes - re-asserted here
    --    so this migration stands alone on a database where 055 predates it.
    INSERT INTO tenants (id, name, slug)
    VALUES (sys_tenant, 'System', 'system')
    ON CONFLICT DO NOTHING;

    -- 2. The six tag groups (000001_initial.sql:253-260, verbatim).
    INSERT INTO tag_groups (id, tenant_id, name, is_collapsible, sort_order)
    SELECT v.id::uuid, sys_tenant, v.name, v.is_collapsible, v.sort_order
      FROM (VALUES
            ('a0000000-0000-0000-0000-000000000001', 'Source',     true, 1),
            ('a0000000-0000-0000-0000-000000000002', 'Status',     true, 2),
            ('a0000000-0000-0000-0000-000000000003', 'Events',     true, 3),
            ('a0000000-0000-0000-0000-000000000004', 'Services',   true, 4),
            ('a0000000-0000-0000-0000-000000000005', 'Engagement', true, 5),
            ('a0000000-0000-0000-0000-000000000006', 'Custom',     true, 6)
           ) AS v(id, name, is_collapsible, sort_order)
    ON CONFLICT (id) DO NOTHING;
    GET DIAGNOSTICS groups_restored = ROW_COUNT;

    -- 3. The 22 shared system tags (000001_initial.sql:263-285, verbatim, same deterministic ids).
    INSERT INTO tags (id, tenant_id, name, color, group_id, is_system)
    SELECT v.id::uuid, sys_tenant, v.name, v.color, v.group_id::uuid, true
      FROM (VALUES
            ('b0000000-0000-0000-0000-000000000001', 'Referral',      '#4CAF50', 'a0000000-0000-0000-0000-000000000001'),
            ('b0000000-0000-0000-0000-000000000002', 'Website',       '#2196F3', 'a0000000-0000-0000-0000-000000000001'),
            ('b0000000-0000-0000-0000-000000000003', 'Cold Call',     '#FF9800', 'a0000000-0000-0000-0000-000000000001'),
            ('b0000000-0000-0000-0000-000000000004', 'Email Campaign','#9C27B0', 'a0000000-0000-0000-0000-000000000001'),
            ('b0000000-0000-0000-0000-000000000005', 'Social Media',  '#E91E63', 'a0000000-0000-0000-0000-000000000001'),
            ('b0000000-0000-0000-0000-000000000006', 'Active',        '#4CAF50', 'a0000000-0000-0000-0000-000000000002'),
            ('b0000000-0000-0000-0000-000000000007', 'Inactive',      '#9E9E9E', 'a0000000-0000-0000-0000-000000000002'),
            ('b0000000-0000-0000-0000-000000000008', 'Won',           '#4CAF50', 'a0000000-0000-0000-0000-000000000002'),
            ('b0000000-0000-0000-0000-000000000009', 'Lost',          '#F44336', 'a0000000-0000-0000-0000-000000000002'),
            ('b0000000-0000-0000-0000-00000000000a', 'Webinar',       '#00BCD4', 'a0000000-0000-0000-0000-000000000003'),
            ('b0000000-0000-0000-0000-00000000000b', 'Demo',          '#FF5722', 'a0000000-0000-0000-0000-000000000003'),
            ('b0000000-0000-0000-0000-00000000000c', 'Meeting',       '#795548', 'a0000000-0000-0000-0000-000000000003'),
            ('b0000000-0000-0000-0000-00000000000d', 'Proposal Sent', '#607D8B', 'a0000000-0000-0000-0000-000000000003'),
            ('b0000000-0000-0000-0000-00000000000e', 'Consulting',    '#3F51B5', 'a0000000-0000-0000-0000-000000000004'),
            ('b0000000-0000-0000-0000-00000000000f', 'Marketing',     '#FF4081', 'a0000000-0000-0000-0000-000000000004'),
            ('b0000000-0000-0000-0000-000000000010', 'Sales',         '#448AFF', 'a0000000-0000-0000-0000-000000000004'),
            ('b0000000-0000-0000-0000-000000000011', 'Support',       '#69F0AE', 'a0000000-0000-0000-0000-000000000004'),
            ('b0000000-0000-0000-0000-000000000012', 'Hot',           '#F44336', 'a0000000-0000-0000-0000-000000000005'),
            ('b0000000-0000-0000-0000-000000000013', 'Warm',          '#FF9800', 'a0000000-0000-0000-0000-000000000005'),
            ('b0000000-0000-0000-0000-000000000014', 'Cold',          '#2196F3', 'a0000000-0000-0000-0000-000000000005'),
            ('b0000000-0000-0000-0000-000000000015', 'New Lead',      '#00E676', 'a0000000-0000-0000-0000-000000000005'),
            ('b0000000-0000-0000-0000-000000000016', 'Follow Up',     '#AA00FF', 'a0000000-0000-0000-0000-000000000005')
           ) AS v(id, name, color, group_id)
    ON CONFLICT (id) DO NOTHING;
    GET DIAGNOSTICS shared_restored = ROW_COUNT;

    -- 4. The seven per-app Free tags (043:13-39, names and colours verbatim). 043 used a random id
    --    with a NOT EXISTS guard on (name, is_system), so that - not an id - is the idempotency key
    --    a re-run must use. The product link below is the only thing that gives these a reader.
    INSERT INTO tags (id, tenant_id, name, color, is_system)
    SELECT gen_random_uuid(), sys_tenant, v.name, v.color, true
      FROM (VALUES
            ('FunnelSwift — Capture Free', '#2563eb'),
            ('FunnelSwift — Kinetic Free', '#7c3aed'),
            ('CoreSwift — Free',           '#059669'),
            ('WorkflowSwift — Free',       '#0ea5e9'),
            ('IncentiveSwift — Free',      '#f59e0b'),
            ('ADASwift — Free',            '#dc2626'),
            ('MissedCall Respondr — Free', '#475569')
           ) AS v(name, color)
     WHERE NOT EXISTS (SELECT 1 FROM tags t WHERE t.name = v.name AND t.is_system = true);
    GET DIAGNOSTICS free_restored = ROW_COUNT;

    -- 5. Sold + Qualified (0014:5-12, verbatim, deterministic ids under the Status group). These ids
    --    are compile-time constants in the app (tag_logic.rs:83-86), so the id MUST be the original.
    INSERT INTO tags (id, tenant_id, group_id, name, color, is_system)
    VALUES ('15698a9a-67fe-5bf1-9aac-1dcd7a1ccd9e', sys_tenant, status_group, 'Qualified', '#4CAF50', true),
           ('3b008e4a-dbc8-5558-8762-2e1787ec7c2c', sys_tenant, status_group, 'Sold',      '#FF9800', true)
    ON CONFLICT (id) DO NOTHING;
    GET DIAGNOSTICS lifecycle_restored = ROW_COUNT;

    -- 6. 0014's rule: assigning Sold removes Qualified (0014:15-27, verbatim). evaluate_tag_rules
    --    (src/tag_logic.rs:27) reads rules from the caller's tenant OR the System tenant, so this row
    --    is live for every workspace.
    IF to_regclass('public.tag_rules') IS NOT NULL THEN
        INSERT INTO tag_rules (id, tenant_id, name, description, trigger_tag_id, action_type,
                               action_tag_id, target_app, is_active)
        VALUES ('dcdd1042-9a01-5797-8e14-d2653825c74c', sys_tenant, 'Sold removes Qualified',
                'When a lead gets the Sold tag, auto-remove the Qualified tag',
                '3b008e4a-dbc8-5558-8762-2e1787ec7c2c', 'remove_tag',
                '15698a9a-67fe-5bf1-9aac-1dcd7a1ccd9e', 'funnelswift', true)
        ON CONFLICT (id) DO NOTHING;
        GET DIAGNOSTICS rule_restored = ROW_COUNT;
    END IF;

    -- 7. Link each product to its tag - the write that makes the reader resolve something. Guarded on
    --    system_tag_id IS NULL so an admin's own link is never clobbered, and matched on
    --    (name, source_app) so a second product with the same name under another app cannot be hit.
    IF to_regclass('public.affiliate_products') IS NOT NULL THEN
        UPDATE affiliate_products p
           SET system_tag_id = t.id, updated_at = NOW()
          FROM (VALUES
                ('Capture Free',            'funnelswift',      'FunnelSwift — Capture Free'),
                ('Kinetic Free',            'funnelswift',      'FunnelSwift — Kinetic Free'),
                ('CoreSwift Free',          'coreswift',        'CoreSwift — Free'),
                ('WorkflowSwift Free',      'workflowswift',    'WorkflowSwift — Free'),
                ('IncentiveSwift Free',     'incentiveswift',   'IncentiveSwift — Free'),
                ('ADASwift Free',           'adaswift',         'ADASwift — Free'),
                ('MissedCall Respondr Free','missedcallrespondr','MissedCall Respondr — Free')
               ) AS m(product_name, source_app, tag_name)
          JOIN tags t ON t.name = m.tag_name AND t.is_system = true
         WHERE p.name = m.product_name
           AND p.source_app = m.source_app
           AND p.system_tag_id IS NULL;
        GET DIAGNOSTICS links_set = ROW_COUNT;
    END IF;

    -- 8. Post-state as NOTICEs only (see the header on why not an assert).
    SELECT count(*) INTO sys_tags FROM tags WHERE is_system = true;
    RAISE NOTICE '060: restored % groups, % shared tags, % free-tier tags, % lifecycle tags, % rules; % new product links',
                 groups_restored, shared_restored, free_restored, lifecycle_restored, rule_restored, links_set;
    RAISE NOTICE '060: is_system tags now % (was 0)', sys_tags;

    -- Every expected tag name must be present, whatever route created it.
    SELECT string_agg(e.name, ', ') INTO missing
      FROM (VALUES ('FunnelSwift — Capture Free'), ('FunnelSwift — Kinetic Free'), ('CoreSwift — Free'),
                   ('WorkflowSwift — Free'), ('IncentiveSwift — Free'), ('ADASwift — Free'),
                   ('MissedCall Respondr — Free'), ('Sold'), ('Qualified')) AS e(name)
     WHERE NOT EXISTS (SELECT 1 FROM tags t WHERE t.name = e.name AND t.is_system = true);
    RAISE NOTICE '060: expected tag names still missing: %', coalesce(missing, '(none)');

    -- Which product each Free tag now routes to - the reader's own predicate.
    IF to_regclass('public.affiliate_products') IS NOT NULL THEN
        SELECT count(*) INTO linked FROM affiliate_products WHERE system_tag_id IS NOT NULL;
        SELECT coalesce(string_agg(x.tag_name || ' -> ' || coalesce(x.pname, '(no product)'), ', '
                                   ORDER BY x.tag_name), '(none)')
          INTO census
          FROM (SELECT t.name AS tag_name,
                       (SELECT p.name FROM affiliate_products p
                         WHERE p.system_tag_id = t.id AND p.is_active = true
                         ORDER BY p.name LIMIT 1) AS pname
                  FROM tags t
                 WHERE t.is_system = true
                   AND t.name IN ('FunnelSwift — Capture Free', 'FunnelSwift — Kinetic Free',
                                  'CoreSwift — Free', 'WorkflowSwift — Free',
                                  'IncentiveSwift — Free', 'ADASwift — Free',
                                  'MissedCall Respondr — Free', 'Sold', 'Qualified')) x;
        RAISE NOTICE '060: tag -> product: %', census;
        RAISE NOTICE '060: products with a system tag: % of % active',
                     linked, (SELECT count(*) FROM affiliate_products WHERE is_active = true);

        SELECT string_agg(k || ' x' || n, ', ') INTO dups
          FROM (SELECT t.name AS k, count(*) AS n
                  FROM affiliate_products p JOIN tags t ON t.id = p.system_tag_id
                 WHERE p.is_active = true
                 GROUP BY t.name HAVING count(*) > 1) d;
        RAISE NOTICE '060: tags routing more than one active product (the reader takes them all): %',
                     coalesce(dups, '(none)');
    END IF;

    SELECT string_agg(v.name, ', ') INTO unlinked
      FROM (VALUES ('FunnelSwift — Capture Free'), ('FunnelSwift — Kinetic Free'), ('CoreSwift — Free'),
                   ('WorkflowSwift — Free'), ('IncentiveSwift — Free'), ('ADASwift — Free'),
                   ('MissedCall Respondr — Free')) AS v(name)
     WHERE NOT EXISTS (SELECT 1 FROM tags t
                        JOIN affiliate_products p ON p.system_tag_id = t.id
                       WHERE t.name = v.name AND p.is_active = true);
    RAISE NOTICE '060: Free tags with no active product behind them: %', coalesce(unlinked, '(none)');
END $mig$;
