env "local" {
  url = getenv("DATABASE_URL")
  # Built from infra/db/postgres (pg_bigm); `make db-up` builds it first.
  dev = "docker+postgres://_/zoo2-postgres-bigm:16/dev?search_path=public"

  schema {
    src = "file://db/schema.sql"
  }

  migration {
    dir = "file://db/migrations"
  }
}
