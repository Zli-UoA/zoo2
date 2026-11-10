-- Modify "attachments" table
ALTER TABLE "attachments" ADD COLUMN "width" integer NULL, ADD COLUMN "height" integer NULL, ADD COLUMN "blurhash" text NULL;
