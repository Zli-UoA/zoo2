env "local" {
  url = getenv("DATABASE_URL")
  dev = "docker://postgres/16/dev?search_path=public"

  schema {
    src = "file://db/schema.sql"
  }

  migration {
    dir = "file://db/migrations"
  }
}
