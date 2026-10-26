-- 117 — a funnel can ask its visitor for a phone number.
--
-- FunnelSwift's public funnel page asked only for a name and an email, while `leads.phone`
-- already existed and the panel renders it: a customer whose follow-up is a phone call had no
-- number to call. The switch is PER FUNNEL and defaults OFF, so every page already shared keeps
-- exactly the shape it has today and only the pages whose owner asks for a number grow a field.
ALTER TABLE funnels ADD COLUMN IF NOT EXISTS ask_phone boolean NOT NULL DEFAULT false;
