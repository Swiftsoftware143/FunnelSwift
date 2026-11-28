-- 150: a card's public address belongs to the WORKSPACE that owns the card, not the whole install.
--
-- Measured 2026-11-28 (kanban t_89f6de9c): `c.zaarhub.com/david` rendered a card owned by the
-- Swift Admin's Workspace (SwiftSoftware), not by ZaarHub, because `kinetic_cards.slug` carried a
-- UNIQUE index on the slug ALONE (`kinetic_cards_slug_key`) while a card belongs to a workspace
-- (`kinetic_cards.tenant_id`). Two consequences, both live:
--   * the custom-domain resolver looked a card up by slug only, so ANY workspace's card could
--     answer on ANY other workspace's branded domain;
--   * two workspaces on one install could never both own `/david` — the second one was shadowed.
--
-- This file makes the ADDRESS scoped exactly like the card:
--   * uniqueness becomes (tenant_id, slug), so two workspaces may hold the same slug and neither
--     shadows the other;
--   * the retired-address table (`kinetic_card_slug_aliases`, migration 136) becomes
--     (tenant_id, old_slug) for the same reason — a retired address is an address too, and a
--     rename in one workspace must not take another workspace's old link.
--
-- The code side (`handlers/kinetic_handler.rs`): every public read (page, vCard, lead form,
-- password unlock) resolves the workspace that CLAIMED the request host first and looks the slug
-- up inside it. Only when that workspace holds no such address does the account-wide lookup run —
-- so links printed before this change keep working — and the fallback is logged, never silent.

-- The slug-alone uniqueness is a table CONSTRAINT on the live install (Postgres backs it with the
-- index of the same name), so it is dropped as a constraint; the plain DROP INDEX covers an
-- install where it was created as a bare index.
ALTER TABLE kinetic_cards DROP CONSTRAINT IF EXISTS kinetic_cards_slug_key;
DROP INDEX IF EXISTS kinetic_cards_slug_key;

CREATE UNIQUE INDEX IF NOT EXISTS kinetic_cards_tenant_slug_key
    ON kinetic_cards (tenant_id, slug);

ALTER TABLE kinetic_card_slug_aliases
    ADD COLUMN IF NOT EXISTS tenant_id uuid;

UPDATE kinetic_card_slug_aliases a
   SET tenant_id = c.tenant_id
  FROM kinetic_cards c
 WHERE c.id = a.card_id
   AND a.tenant_id IS DISTINCT FROM c.tenant_id;

ALTER TABLE kinetic_card_slug_aliases
    ALTER COLUMN tenant_id SET NOT NULL;

ALTER TABLE kinetic_card_slug_aliases
    DROP CONSTRAINT IF EXISTS kinetic_card_slug_aliases_pkey;

ALTER TABLE kinetic_card_slug_aliases
    ADD CONSTRAINT kinetic_card_slug_aliases_pkey PRIMARY KEY (tenant_id, old_slug);
