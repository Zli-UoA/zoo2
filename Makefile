.PHONY: migrate migrate-diff migrate-validate migrate-status db-up db-down db-restart db-reset backend-dev backend-build frontend-install frontend-dev frontend-build frontend-check

backend-dev:
	cd backend && DEV_AUTH_ENABLED=true cargo run

backend-build:
	cd backend && cargo build --release

frontend-install:
	cd frontend && pnpm install

frontend-dev:
	cd frontend && pnpm dev

frontend-build:
	cd frontend && pnpm build

frontend-check:
	cd frontend && pnpm check

migrate:
	$(MAKE) -C infra migrate

migrate-diff:
	@if [ "$(origin name)" != "command line" ] || [ -z "$(name)" ]; then \
		echo "name is required (example: make migrate-diff name=add_example)" >&2; \
		exit 1; \
	fi
	$(MAKE) -C infra migrate-diff name=$(name)

migrate-validate:
	$(MAKE) -C infra migrate-validate

migrate-status:
	$(MAKE) -C infra migrate-status

db-up:
	$(MAKE) -C infra db-up

db-down:
	$(MAKE) -C infra db-down

db-restart:
	$(MAKE) -C infra db-restart

db-reset:
	$(MAKE) -C infra db-reset
