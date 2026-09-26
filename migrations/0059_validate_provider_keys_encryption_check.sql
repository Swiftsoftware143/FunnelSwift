-- Migration 059: validate the provider_keys encryption CHECK, as the live database has it.
--
-- WHY THIS FILE EXISTS (kanban t_f7341aa9, measured 2026-09-26)
--   050_provider_keys_encrypted_at_rest.sql adds
--       provider_keys_api_key_encrypted CHECK (api_key = '' OR api_key LIKE 'enc:v1:%') NOT VALID
--   and its own comment says the constraint is validated once after the one-off backfill of legacy
--   plaintext rows. The live database is VALID (pg_constraint.convalidated = true), a fresh
--   database was NOT VALID -- real drift that neither a table count nor a column count shows, and
--   exactly the kind of difference that made from-zero environments a false oracle for schema
--   questions.
--
--   A fresh build has no legacy plaintext rows (it never had an unencrypted writer), so the
--   validation scan is trivially satisfied there. On live the guard finds convalidated = true and
--   the statement does not run at all.
--
--   The ordering is why this cannot live in 032: 050 sorts after 032, so at 032's position the
--   constraint does not exist yet and the guard would skip on a fresh build too. 059 sorts last.
--
-- NO SEMICOLONS IN THIS HEADER, deliberately (the deploy staging path splits on the statement
--   separator, as 050 records).

DO $mig$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_constraint
              WHERE conname = 'provider_keys_api_key_encrypted'
                AND conrelid = 'public.provider_keys'::regclass
                AND NOT convalidated) THEN
    ALTER TABLE provider_keys VALIDATE CONSTRAINT provider_keys_api_key_encrypted;
  END IF;
END $mig$;
