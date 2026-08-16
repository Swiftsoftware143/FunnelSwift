-- Allow commissions to accrue independently of a lead record.
-- A conversion may reference a lead when one exists, but must not be blocked
-- when a sale is attributed to an affiliate without a matching lead row.
ALTER TABLE affiliate_commissions ALTER COLUMN lead_id DROP NOT NULL;
