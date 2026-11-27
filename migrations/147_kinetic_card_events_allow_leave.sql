-- The Kinetic card page has always reported an EXIT: its inline tracker sends
-- event_type = 'leave' when the visitor hides the page (visibilitychange -> hidden).
--
-- The vocabulary created in 028 never listed that word, so Postgres refused every one of those
-- inserts (`23514 kinetic_card_events_event_type_check`), `track_card_event` swallowed the error
-- (`unwrap_or_default`) and still answered 200 — the log line was the only trace. Measured live
-- 2026-11-27 13:21:28Z: a real card visit recorded view + click and lost its exit, one ERROR per
-- visitor, and the operator's card analytics could never show anybody leaving.
--
-- Widen the vocabulary so the event the page already sends is storable. The DROP makes a re-run
-- (and a from-zero build, where 028 has just created the constraint inline) a no-op.
ALTER TABLE kinetic_card_events
    DROP CONSTRAINT IF EXISTS kinetic_card_events_event_type_check;

ALTER TABLE kinetic_card_events
    ADD CONSTRAINT kinetic_card_events_event_type_check
    CHECK (event_type IN (
        'view', 'click', 'share', 'qr_scan', 'nfc_tap',
        'button_click', 'link_click', 'form_submit', 'leave'
    ));
