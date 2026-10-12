.PHONY: migrate migrate-diff migrate-status db-up db-down db-restart db-reset

-include ../.env

export POSTGRES_USER POSTGRES_PASSWORD POSTGRES_DB DATABASE_URL

migrate:
	atlas migrate apply --config file://db/atlas.hcl --env local

migrate-diff:
	atlas migrate diff --config file://db/atlas.hcl --env local $(name)

migrate-status:
	atlas migrate status --config file://db/atlas.hcl --env local

db-up:
db-down:

db-restart:

db-reset:
