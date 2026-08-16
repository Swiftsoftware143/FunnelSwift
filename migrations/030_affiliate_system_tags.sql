-- Migration 030: Affiliate products linked to system tags (tag-based affiliate routing)
-- David's model:
--   * Each Swift product (CoreSwift, FunnelSwift, IncentiveSwift, MultiDirectory, etc.) is an
--     affiliate product. Admin assigns a system tag to each product.
--   * The system tag marks a lead as having acquired the FREE PLAN of that product (upsells
--     happen inside each respective app).
--   * Attribution is tracked on the USER ACCOUNT the lead flows through (leads.created_by).
--     If that user signed up for the affiliate program (affiliates.user_id), a pending
--     commission is recorded; admin / non-affiliate users attribute to no affiliate.

-- Link each affiliate product to a system tag (the routing signal)
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS system_tag_id UUID REFERENCES tags(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_affiliate_products_tag ON affiliate_products(system_tag_id);

-- Make the commission ledger product-aware so tag-based attribution records WHICH product converted
ALTER TABLE affiliate_commissions ADD COLUMN IF NOT EXISTS product_id UUID REFERENCES affiliate_products(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_affiliate_commissions_product ON affiliate_commissions(product_id);

-- Track which user account a lead flows through (the affiliate attribution anchor)
ALTER TABLE leads ADD COLUMN IF NOT EXISTS created_by UUID REFERENCES users(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_leads_created_by ON leads(created_by);

-- First-class link: which user account this affiliate is (commission is tracked on the user account)
ALTER TABLE affiliates ADD COLUMN IF NOT EXISTS user_id UUID;
CREATE INDEX IF NOT EXISTS idx_affiliates_user ON affiliates(user_id);
