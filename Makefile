.PHONY: migrate migrate-diff migrate-validate migrate-status db-up db-down db-restart db-reset

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
