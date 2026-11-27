-- 149: the daily rollup's key must be NULL-safe, or every event mints a new row.
--
-- Measured 2026-11-27 on the live install: `kinetic_card_daily_stats` held 29 rows for 3 keys,
-- 15 of them for ONE card on ONE day, each carrying a single view. The writer upserts with
-- `ON CONFLICT (card_id, stat_date, utm_source, utm_medium, utm_campaign)`, and every one of those
-- utm columns is NULL for direct traffic — Postgres treats NULLs as DISTINCT in a plain unique
-- index, so the conflict never fired and each event inserted a fresh row instead of incrementing
-- the day's row. The per-card Analytics chart reads the table row by row (`ORDER BY stat_date DESC
-- LIMIT 30`), so it drew one point per event instead of one per day and could not reach 30 days
-- back at all.
--
-- This file (1) folds the existing duplicates into one row per key — summing the counters, never
-- dropping the numbers — and (2) adds the NULL-safe unique index the writer now conflicts on.

WITH agg AS (
    SELECT card_id,
           stat_date,
           COALESCE(utm_source, '')   AS s,
           COALESCE(utm_medium, '')   AS m,
           COALESCE(utm_campaign, '') AS c,
           MIN(id::text)::uuid        AS keep_id,
           SUM(views)::bigint         AS views,
           SUM(clicks)::bigint        AS clicks,
           SUM(shares)::bigint        AS shares
      FROM kinetic_card_daily_stats
     GROUP BY 1, 2, 3, 4, 5
    HAVING COUNT(*) > 1
)
UPDATE kinetic_card_daily_stats d
   SET views  = a.views,
       clicks = a.clicks,
       shares = a.shares
  FROM agg a
 WHERE d.id = a.keep_id;

WITH agg AS (
    SELECT card_id,
           stat_date,
           COALESCE(utm_source, '')   AS s,
           COALESCE(utm_medium, '')   AS m,
           COALESCE(utm_campaign, '') AS c,
           MIN(id::text)::uuid        AS keep_id
      FROM kinetic_card_daily_stats
     GROUP BY 1, 2, 3, 4, 5
    HAVING COUNT(*) > 1
)
DELETE FROM kinetic_card_daily_stats d
 USING agg a
 WHERE d.card_id = a.card_id
   AND d.stat_date = a.stat_date
   AND COALESCE(d.utm_source, '') = a.s
   AND COALESCE(d.utm_medium, '') = a.m
   AND COALESCE(d.utm_campaign, '') = a.c
   AND d.id <> a.keep_id;

CREATE UNIQUE INDEX IF NOT EXISTS kinetic_card_daily_stats_null_safe_key
    ON kinetic_card_daily_stats (
        card_id,
        stat_date,
        COALESCE(utm_source, ''),
        COALESCE(utm_medium, ''),
        COALESCE(utm_campaign, '')
    );
