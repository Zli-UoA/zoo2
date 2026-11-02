-- Modify "workspace_emojis" table
ALTER TABLE "workspace_emojis" ADD COLUMN "category" text NOT NULL DEFAULT '', ADD COLUMN "aliases" jsonb NOT NULL DEFAULT '[]';
