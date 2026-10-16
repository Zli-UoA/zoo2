-- Modify "users" table
ALTER TABLE "users" ADD COLUMN "oidc_iss" text NULL, ADD COLUMN "oidc_sub" text NULL, ADD COLUMN "affiliation" character varying(32) NOT NULL DEFAULT 'unknown', ADD CONSTRAINT "users_oidc_iss_sub_key" UNIQUE ("oidc_iss", "oidc_sub");
