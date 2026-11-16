-- 136_kinetic_card_slug_aliases.sql
-- A card's OLD public address keeps working after the customer renames it.
--
-- `kinetic_cards.slug` is UNIQUE and is the card's whole public identity (`/k/<slug>`, `/b/<slug>`,
-- `/c|m|f|h/<slug>`). Since 2026-11-16 the card editor lets a customer TYPE a new address
-- (PUT /api/v1/kinetic/cards/:id). Until now the previous address simply stopped matching: every
-- business card, QR code, email signature and social post already printed with `/k/<old>` answered
-- "Card not found" the moment the rename saved. Measured 2026-11-16: no alias/redirect table
-- existed for cards (`information_schema.tables` held none matching alias/redirect), so nothing
-- recorded the old address at all.
--
-- This table records `old_slug -> card_id`. `render_card` reads it ONLY when no card currently owns
-- the requested address, and answers a 302 to the card's CURRENT slug. 302 rather than 301: a
-- retired address may legitimately be claimed by a different card later, and that card wins because
-- the live row is looked up first. ON DELETE CASCADE means deleting a card also retires the
-- addresses it used to answer on, so a deleted card's old links cannot point at a live page.
CREATE TABLE IF NOT EXISTS kinetic_card_slug_aliases (
    old_slug   text        PRIMARY KEY,
    card_id    uuid        NOT NULL REFERENCES kinetic_cards(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- One card may retire several addresses over its life (a → b → c keeps a and b); this is the read
-- used when auditing or cleaning a single card's history.
CREATE INDEX IF NOT EXISTS idx_kinetic_card_slug_aliases_card
    ON kinetic_card_slug_aliases (card_id);
