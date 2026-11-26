#!/usr/bin/env python3
"""Exercise Atlas partial-revision recovery against a disposable PostgreSQL DB."""
import os
from pathlib import Path
import subprocess
import tempfile
from urllib.parse import urlsplit, urlunsplit
from uuid import uuid4


def run(args, *, env=None, succeeds=True):
    result = subprocess.run(args, env=env, text=True, capture_output=True)
    if (result.returncode == 0) != succeeds:
        # Do not echo command arguments: database URLs may contain credentials.
        raise RuntimeError(result.stderr[-3000:] + result.stdout[-3000:])
    return result.stdout


admin_url = os.environ["DATABASE_URL"]
name = "zoo_migration_recovery_" + uuid4().hex
parts = urlsplit(admin_url)
url = urlunsplit(parts._replace(path="/" + name))
env = dict(os.environ, DATABASE_URL=url)
wrapper = str(Path(__file__).with_name("migrate.sh").resolve())
run(["psql", "-Xq", admin_url, "-v", "ON_ERROR_STOP=1", "-c", "CREATE DATABASE " + name])
try:
    with tempfile.TemporaryDirectory() as directory:
        Path(directory, "20261128000002_legacy.sql").write_text(
            "CREATE TABLE apub_remote_recipients(post_id text);\n")
        migration_dir = "file://" + directory
        run(["atlas", "migrate", "hash", "--dir", migration_dir])
        run(["atlas", "migrate", "apply", "--dir", migration_dir, "--url", url])
        old = Path(directory, "20261128000003_reply_counters.sql")
        old.write_text("-- atlas:txmode none\n\nCREATE INDEX CONCURRENTLY recipient_parent ON apub_remote_recipients(parent_id);\n" + "SELECT 1;\n" * 6)
        migration_dir = "file://" + directory
        run(["atlas", "migrate", "hash", "--dir", migration_dir])
        failure = subprocess.run(["atlas", "migrate", "apply", "--dir", migration_dir, "--url", url], text=True, capture_output=True)
        assert failure.returncode and "parent_id" in failure.stderr, failure.stderr[-2000:]
        partial = run(["psql", "-XqAt", url, "-v", "ON_ERROR_STOP=1", "-c",
                       "SELECT applied,total FROM atlas_schema_revisions.atlas_schema_revisions WHERE version='20261128000003'"])
        assert partial.strip() == "0|7", partial
        old.unlink()
        Path(directory, "20261128020003_reply_counters.sql").write_text(
            "ALTER TABLE apub_remote_recipients ADD COLUMN parent_id text;\n"
            "CREATE INDEX recipient_parent ON apub_remote_recipients(parent_id);\n")
        run(["atlas", "migrate", "hash", "--dir", migration_dir])
        run([wrapper, "migrate", "apply", "--dir", migration_dir, "--url", url], env=env)
        # A second invocation is a normal no-op, not another recovery attempt.
        run([wrapper, "migrate", "apply", "--dir", migration_dir, "--url", url], env=env)
    actual = run(["psql", "-XqAt", url, "-v", "ON_ERROR_STOP=1", "-c",
                  "SELECT version,applied=total FROM atlas_schema_revisions.atlas_schema_revisions ORDER BY version"])
    assert actual.splitlines() == ["20261128000002|t", "20261128000003|t", "20261128020003|t"], actual
    print("Atlas recovered the failed empty revision and applied the renumbered migration")
finally:
    run(["psql", "-Xq", admin_url, "-v", "ON_ERROR_STOP=1", "-c", "DROP DATABASE " + name])
