-- 143_funnel_qr_codes.sql — a QR code for a FUNNEL page, not only for a kinetic card.
--
-- WHY (cron 2026-11-24). Every plan carries a `max_qr_codes` allowance (Capture Free: 5) and the
-- dashboard counts "QR codes", but a QR could only be minted for a kinetic CARD. A workspace on the
-- default Capture Free plan has `max_cards = 0`, so its QR allowance could not be spent at all — a
-- customer printing a flyer had no way to turn their funnel page into a scannable code. The funnel
-- is the one public surface a free workspace can build, so the allowance was unusable exactly where
-- it was most useful (offline -> online capture for a local business).
--
-- The table already carries the tenant and the plan gate counts its rows, so a funnel QR is the
-- same object with the other end of the link: `card_id` becomes nullable and the two ends are
-- mutually exclusive. Idempotency is per end (one QR per card, one per funnel in a workspace), kept
-- by a partial unique index rather than by the handler alone.

ALTER TABLE kinetic_qr_codes ALTER COLUMN card_id DROP NOT NULL;

ALTER TABLE kinetic_qr_codes ADD COLUMN IF NOT EXISTS funnel_id uuid;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'kinetic_qr_codes_funnel_id_fkey'
    ) THEN
        ALTER TABLE kinetic_qr_codes
            ADD CONSTRAINT kinetic_qr_codes_funnel_id_fkey
            FOREIGN KEY (funnel_id) REFERENCES funnels(id) ON DELETE CASCADE;
    END IF;
END $$;

-- Exactly one of the two ends. A row that names neither cannot build a target URL, and a row that
-- names both would answer "which link does this code open?" with whichever the reader checked first.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'kinetic_qr_codes_one_target'
    ) THEN
        ALTER TABLE kinetic_qr_codes
            ADD CONSTRAINT kinetic_qr_codes_one_target
            CHECK ((card_id IS NOT NULL) <> (funnel_id IS NOT NULL));
    END IF;
END $$;

-- One QR per funnel per workspace: pressing the button twice returns the same code instead of
-- spending a second slot of the plan's allowance.
CREATE UNIQUE INDEX IF NOT EXISTS kinetic_qr_codes_one_per_funnel
    ON kinetic_qr_codes (tenant_id, funnel_id)
    WHERE funnel_id IS NOT NULL;
