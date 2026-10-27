.PHONY: migrate migrate-diff migrate-validate migrate-status db-up db-down db-restart db-reset backend-dev backend-build test test-unit test-e2e frontend-install frontend-dev frontend-build frontend-check frontend-test frontend-e2e

test: test-unit test-e2e

# ユニットテスト (DB 不要)
test-unit:
	cd backend && cargo test --lib

# E2E: 本物の PostgreSQL + Electric を起動し、migration 適用後に tests/ を実行する (Docker / atlas 必須)
test-e2e:
	$(MAKE) -C infra db-up
	$(MAKE) -C infra migrate
	cd backend && DEV_AUTH_ENABLED=true cargo test --test '*'

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

frontend-test:
	cd frontend && pnpm test

# E2E スモーク: PostgreSQL + Electric + バックエンドを起動しておく (起動→送信→反映)
frontend-e2e:
	cd frontend && pnpm test:e2e

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

.PHONY: sdk-check sdk-test
sdk-check:
	frontend/node_modules/.bin/tsc --project sdk/tsconfig.json

sdk-test:
	node --experimental-strip-types --test sdk/src/index.test.ts
