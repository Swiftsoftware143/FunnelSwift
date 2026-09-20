-- Plan gating for Kinetic card themes + templates.
--
-- New plan feature key `premium_themes` (boolean) on `plans.features`.
--
-- Gating rule (mirrored in src/features.rs):
--   * FREE for every plan : themes midnight / ocean / rose, templates bio_*
--   * requires premium_themes : themes cyber_dark, sunset_kinetic,
--     emerald_glass, ghost_white, gold_premium; templates biz_*, page_* and the
--     rest of the non-bio_* catalogue
-- Semantics: no active plan -> allow; key absent/FALSE -> locked (402);
-- key TRUE -> allow.
--
-- Only the paid plans that include the Kinetic theme/template library get the
-- key; every other plan keeps it absent (= locked).
UPDATE plans
SET features = COALESCE(features, '{}'::jsonb) || jsonb_build_object('premium_themes', true),
    updated_at = now()
WHERE name IN ('Kinetic Pro', 'Suite', 'Agency / Scale');
