.PHONY: migrate migrate-diff migrate-validate migrate-status db-up db-down db-restart db-reset

-include ../.env

POSTGRES_USER ?= root
POSTGRES_PASSWORD ?= password
POSTGRES_DB ?= zoo
POSTGRES_PORT ?= 5432
ELECTRIC_VERSION ?= 1.8.1
ELECTRIC_PORT ?= 3000
DATABASE_URL ?= postgres://$(POSTGRES_USER):$(POSTGRES_PASSWORD)@localhost:$(POSTGRES_PORT)/$(POSTGRES_DB)?sslmode=disable
ELECTRIC_URL ?= http://localhost:$(ELECTRIC_PORT)

export POSTGRES_USER POSTGRES_PASSWORD POSTGRES_DB POSTGRES_PORT DATABASE_URL
export ELECTRIC_VERSION ELECTRIC_PORT ELECTRIC_URL

migrate:
	atlas migrate apply --config file://db/atlas.hcl --env local

migrate-diff:
	@if [ "$(origin name)" != "command line" ] || [ -z "$(name)" ]; then \
		echo "name is required (example: make migrate-diff name=add_example)" >&2; \
		exit 1; \
	fi
	atlas migrate diff --config file://db/atlas.hcl --env local $(name)

migrate-validate:
	atlas migrate validate --config file://db/atlas.hcl --env local

migrate-status:
	atlas migrate status --config file://db/atlas.hcl --env local

db-up:
	docker compose -f db/compose.yml up -d

db-down:
	docker compose -f db/compose.yml down

db-restart:
	docker compose -f db/compose.yml restart

db-reset:
	docker compose -f db/compose.yml down --volumes
