-- A funnel's public page is addressed by SLUG ALONE (`/funnel/<slug>`, no tenant in the path), so a
-- slug has to be URL-safe and unique ACROSS the fleet. It was neither: `create_funnel` derived it with
-- `name.to_lowercase().replace(' ', "-")`, so
--   * "Sam's Funnel"     -> `sam's-funnel`          (an apostrophe in a public URL),
--   * "Q3 / Q4 Promo"    -> `q3-/-q4-promo`         (a SLASH: the path splits, `%2F` is not routed
--                                                    by every edge, so the page is unreachable),
--   * two workspaces that both call a funnel "My Funnel" shared `my-funnel`, and the page resolves by
--     slug with no tenant, so one workspace's name answered the other's URL.
-- The writer now derives the slug with the app's ONE transform (`security::display_name::slugify`)
-- and suffixes a collision (`-2`, `-3`, …). This migration makes the table able to enforce that, and
-- repairs any row written by the old transform. It is idempotent: every statement may be re-run.
--
-- Measured 2026-10-17 before writing it: `funnels` held 0 rows, so the repair arms below are a
-- no-op on the live database and exist for any environment that has rows.

-- 1. Repair a slug the old transform left non URL-safe (lowercase, everything outside [a-z0-9_-]
--    collapsed to one dash, no leading/trailing dash). A name of only punctuation yields an EMPTY
--    slug, so that case gets a stable fallback unique per row.
UPDATE funnels
SET slug = NULLIF(trim(both '-' FROM regexp_replace(regexp_replace(lower(name), '[^a-z0-9_-]+', '-', 'g'), '-+', '-', 'g')), '')
WHERE slug IS NULL OR slug !~ '^[a-z0-9_-]+$';

UPDATE funnels
SET slug = 'funnel-' || left(id::text, 6)
WHERE slug IS NULL OR slug = '';

-- 2. De-duplicate before the constraint exists. The suffix is the row's own id prefix, so this can
--    never collide with another row's repaired slug (`-2` suffixes can, hence not used here).
WITH ranked AS (
    SELECT id, slug, row_number() OVER (PARTITION BY slug ORDER BY created_at, id) AS rn
    FROM funnels
    WHERE slug IS NOT NULL
)
UPDATE funnels f
SET slug = r.slug || '-' || left(f.id::text, 6)
FROM ranked r
WHERE f.id = r.id AND r.rn > 1;

-- 3. The constraint the writer's `ON CONFLICT (slug)` inference needs. A plain UNIQUE index permits
--    any number of NULL slugs (Postgres treats NULLs as distinct), so a legacy row without a link is
--    still legal.
CREATE UNIQUE INDEX IF NOT EXISTS uq_funnels_slug ON funnels(slug);
