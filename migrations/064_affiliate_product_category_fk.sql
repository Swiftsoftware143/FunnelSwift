-- Migration 064: affiliate_products.category_id gets the foreign key its two sibling pointers
-- (plan_id, system_tag_id) already have - ON DELETE SET NULL, so deleting a category DETACHES the
-- products that referenced it instead of stranding them on an id that resolves to nothing.
--
-- Card: kanban t_5b835f2e (found while closing t_a0214025, the Affiliate Product Category control).
-- Context: /opt/swift/audits/t_a0214025/REPORT.md section "Adjacent findings" item 3.
--
-- MEASURED on the live `funnelswift` database, 2026-09-26 (pg_constraint read directly, never inferred):
--   * affiliate_products carries exactly three FKs today - plan_id (confdeltype 'n' = SET NULL),
--     system_tag_id ('n' = SET NULL), tenant_id ('c' = CASCADE). category_id carries NONE.
--       SELECT ... FROM pg_constraint WHERE confrelid = 'product_categories'::regclass -- 0 rows
--     and affiliate_products.category_id is the ONLY column in the whole schema whose name contains
--     "category" (pg_class/pg_attribute census), so this pointer has exactly one child column.
--   * the column is nullable (attnotnull = f), so SET NULL is available;
--   * the card's own dangling census is 0 rows:
--       SELECT count(*) FROM affiliate_products ap WHERE ap.category_id IS NOT NULL
--         AND NOT EXISTS (SELECT 1 FROM product_categories pc WHERE pc.id = ap.category_id)  -> 0
--     and 0 of the 11 live products carry a category at all (SELECT count(*) FILTER (WHERE
--     category_id IS NOT NULL) -> 0), so this migration touches no live row.
--
-- WHY THE EDGE IS LOAD-BEARING NOW: t_a0214025 wired the Affiliate Product modal's Category control
-- to write this column, so the value is produced by a real operator action - and
-- product_category_handler::delete_category (src/handlers/product_category_handler.rs:154-167) is a
-- bare `DELETE FROM product_categories WHERE id = $1 AND tenant_id = $2` with no reference check.
-- Before this edge, deleting a category left every referencing product pointing at an id that no
-- longer exists: the row stayed in the list with category_name NULL (the response's LEFT JOIN
-- tolerates it) and nothing anywhere reported it - a silent, permanent strand.
--
-- WHY SET NULL AND NOT CASCADE / RESTRICT - read from the READERS, not from taste:
--   * every read of the pointer is a projection, never a filter: grep -rn category_id src/ finds the
--     LEFT JOIN in affiliate_product_handler::list_affiliate_products plus INSERT/UPDATE binds only -
--     NO select scopes rows BY category_id. The product is workspace content that outlives a taxonomy
--     entry, so it must be preserved: CASCADE would delete real products as a side effect of tidying
--     a category, which is worse than the defect being fixed.
--   * a product with no category is already a first-class rendered state: the list emits
--     `category_name: null`, the screen shows `-`, and the modal offers an empty choice - NULL is the
--     value the app already means by "no category".
--   * RESTRICT/NO ACTION would turn an app delete into a 500 with no product named in it, and would
--     make the fleet's shared system-tenant taxonomy (7 of the 15 live rows) undeletable; blocking
--     the delete was not asked for and no reader needs it.
--   * this is the exact shape the two sibling pointers already use (026 plan_id, 030 system_tag_id,
--     both confdeltype 'n'), so the table now states one rule for all three of its soft pointers.
--
-- `NOT VALID` is deliberately NOT used: the live census is 0 dangling rows, so the constraint is
-- added VALIDATED - the stronger statement. The UPDATE below is only a guard for the window between
-- the measurement and this deploy (a no-op on the measured state): it clears any category_id whose
-- category row is already gone, so ADD CONSTRAINT cannot fail and cannot leave the app booting green
-- on a schema it never applied (the failure mode carded as t_c3823fd1).

UPDATE affiliate_products ap
   SET category_id = NULL
 WHERE ap.category_id IS NOT NULL
   AND NOT EXISTS (SELECT 1 FROM product_categories pc WHERE pc.id = ap.category_id);

ALTER TABLE affiliate_products
  ADD CONSTRAINT affiliate_products_category_id_fkey
  FOREIGN KEY (category_id) REFERENCES product_categories(id) ON DELETE SET NULL;

-- The arm above makes every product_categories DELETE look up its children, so the child column gets
-- the index the sibling pointer already has (`idx_affiliate_products_tag` on system_tag_id, migration
-- 030). 11 products today, so this is hygiene, not a performance fix - it exists so the SET NULL sweep
-- does not become a sequential scan of affiliate_products the first time a category with products is
-- deleted. `IF NOT EXISTS` because an index is addable out of band and a replay must not fail.
CREATE INDEX IF NOT EXISTS idx_affiliate_products_category ON affiliate_products(category_id);
