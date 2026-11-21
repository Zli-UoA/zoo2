# syntax=docker/dockerfile:1
# Local/CI PostgreSQL with pg_bigm (full-text search). zoo2 is public, but the
# images in ghcr.io/zli-uoa are private, so compose and CI build this one here.
# Keep it identical to Zli-UoA/zoo2-infra images/postgres-bigm-dev (preview)
# and in step with images/postgres-bigm (production CNPG).
ARG BASE=postgres:16.15-trixie@sha256:f9a7b9ebf6f56201f949db745e47bc423c0f0f8294bafe991cc10bf74b029657

FROM ${BASE} AS build
ARG PG_BIGM_VERSION=1.2-20250903
ARG PG_BIGM_SHA256=4d4fb48161e3f9e99207851299379c83b3e5d1d3c1a07c35f6a3153d577b3142
RUN set -eux; \
    pg_version=$(dpkg-query -W -f='${Version}' postgresql-16); \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl gcc libicu-dev make \
      "postgresql-server-dev-16=${pg_version}"; \
    curl -fsSL -o /tmp/pg_bigm.tar.gz \
      "https://github.com/pgbigm/pg_bigm/archive/refs/tags/v${PG_BIGM_VERSION}.tar.gz"; \
    echo "${PG_BIGM_SHA256}  /tmp/pg_bigm.tar.gz" | sha256sum -c -; \
    mkdir /src; tar -xzf /tmp/pg_bigm.tar.gz -C /src --strip-components=1; \
    make -C /src USE_PGXS=1 with_llvm=no; \
    make -C /src USE_PGXS=1 with_llvm=no install DESTDIR=/out

FROM ${BASE}
COPY --from=build /out/ /
