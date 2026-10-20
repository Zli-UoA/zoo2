# GHCR images

Issue #73 publishes application images; deploying them to k3s and managing Preview namespaces belong to the subsequent deployment issues. Existing Compose configuration remains available.

## Image contract

All images use `ghcr.io/zli-uoa/zoo2-<component>` and `linux/amd64`.

| Event | frontend | backend / migrate |
| --- | --- | --- |
| main push | `<40-character SHA>`, `latest` with production OIDC; `preview-<SHA>`, `preview` with development authentication | `<SHA>`, `latest` |
| Same-repository PR opened / synchronized / reopened | `pr-<number>-<head SHA>` with development authentication when changed; otherwise `preview` | `pr-<number>-<head SHA>` when changed; otherwise `latest` |
| Fork PR | Publication skipped | Publication skipped |

The plan job outputs `sha`, `tags` (a component-to-tag JSON object), and the build matrix. Its Actions summary records the selected references. The PR diff is cumulative against the base, including original paths for renamed files. Source files and component Dockerfiles/ignore files trigger their component; nginx configuration triggers frontend; database definitions trigger migrate. Shared image scripts, image actions, the publication workflow, or a root Docker ignore change trigger every component. Diffs reaching GitHub's 3,000-file limit also build every component.

Bootstrap all main images before using unchanged-component fallbacks. `latest` and `preview` are mutable aliases; a deployment controller should resolve them to digests when recording a release. After every component build succeeds, a single promote job checks the current main head once and moves all aliases together, so `latest` never mixes commits. Old main and stale/closed PR runs cannot overwrite current aliases or publish obsolete PR builds, respectively. Images pushed by a build still running when its PR closes are reclaimed by the daily cleanup.

## Repository setup

Configure these **Repository Variables**, not secrets, for the existing Kanidm client. They become public frontend build output. Do not invent production values.

| Variable | Requirement |
| --- | --- |
| `VITE_OIDC_ISSUER` | Required, existing Kanidm issuer |
| `VITE_OIDC_CLIENT_ID` | Required, existing client ID |
| `VITE_OIDC_REDIRECT_URI` | Optional; application defaults to the origin plus `/auth/callback` |
| `VITE_OIDC_SCOPE` | Optional; application default is `openid profile email` |
| `VITE_OIDC_POST_LOGOUT_REDIRECT_URI` | Optional; application default |

Production frontend publication fails if issuer/client ID is empty or whitespace. Preview frontend builds explicitly clear all five values. `VITE_API_BASE_URL` is empty to use same-origin `/api` routing. Existing authentication readiness requirements still apply before production deployment.

Publication and cleanup use the repository `GITHUB_TOKEN` with `packages: write`. Images carry the `org.opencontainers.image.source` repository label. For existing packages, check package settings: link the source repository, allow this repository's Actions access, and grant the repository **Admin** package access for deletion. Organization policies must permit image publication. A missing package is skipped; permission failures fail the run. Package visibility and k3s pull credentials are configured separately; publication does not make a private package publicly pullable.

## Retention and operation

`Clean up images` runs on PR close, daily at 18:23 UTC (03:23 JST), and manually. Manual runs default to `dry_run: true`; review the Actions summary before selecting `false`. Close and scheduled runs delete candidates automatically.

- Keep the newest 2 image versions per component for each open PR.
- Keep the newest 15 production SHA versions and 15 shared Preview SHA versions per component, sorted by creation time.
- Remove all generations belonging to closed PRs. Close events target their PR; daily runs also catch missed close events.
- Keep `latest`, `preview`, unknown tags, and any version carrying at least one retained tag. A version is deleted only when all its tags are disposable.
- Delete untagged versions after 24 hours, preserving platform and attestation manifests recursively referenced by retained OCI indexes. A manifest missing from the registry (404) references nothing; other inspection failures abort cleanup rather than assume a manifest is disposable.
- Open PRs are listed once per package. Before each deletion, recheck the version's tags and, for closed-PR tags, the PR state. A version whose tags changed or whose PR was reopened is kept along with its referenced manifests, and cleanup continues. Already deleted versions (404) are harmless; other API errors fail the run, and the Actions summary still lists the deletions completed before the failure.

Image builds are serialized per branch or PR (`ghcr-images-<ref>`) and cleanup runs in its own `ghcr-cleanup` group, both with `queue: max` so pending runs are not replaced. GitHub permits up to 100 queued runs. A build can reference an existing untagged manifest from a new index, so cleanup defers untagged deletion while any `Publish images` run is queued or in progress; tagged deletions still proceed because each is rechecked. Do not publish to these packages from a writer outside `Publish images`.

Buildx uses component-specific GitHub Actions layer caches. Backend additionally restores Cargo registry, git, and target cache mounts using `actions/cache` and `buildkit-cache-dance`. The cache key is the hash of `Cargo.lock` and the backend Dockerfile, so it is saved once per dependency set; on an exact hit, extraction and saving are skipped to stay within the 10 GB repository cache limit. The release executable is copied out of the target cache mount before the Distroless stage copies it. Local target directories and environment files are excluded from the backend build context.

## Validation and acceptance

Local verification on 2026-10-20: all 18 policy/API tests passed; backend, migrate, Preview frontend, and frontend with synthetic OIDC build arguments built successfully. The backend executable is approximately 14 MiB and `file` reports `stripped`. It starts in Distroless and reaches database connection initialization; no database was provided for this smoke check. `git diff --check` passed. Actionlint passed with only its unsupported `concurrency.queue` key excluded; that key was checked against GitHub's official queue documentation.

Actual GHCR push/deletion, Actions cache restoration, production Kanidm values, and k3s acceptance remain unverified. Repository Variables were empty when inspected during planning; no production values or package settings were changed.

Run the policy tests with:

```sh
node .github/scripts/images.test.mjs
```

They exercise component selection, head SHA tagging, fork/stale/closed PR skips, rename/truncated diffs, retention boundaries, multi-tag protection, untagged age, manifest references, and API failure handling. Both workflows run these tests before their operation.

Build the three Dockerfiles from the repository root to validate container output. For production frontend build acceptance, pass the actual configured OIDC build arguments; an empty-argument build validates only Preview output.

After merge and repository setup, verify separately on GitHub:

1. Main publishes all immutable tags and current aliases; verify production OIDC and Preview development-auth builds.
2. A same-repository PR changing only one component publishes only its image with its head SHA; inspect fallback references. Fork PRs do not publish.
3. Repeated backend builds restore both Docker layers and Rust cache mounts in Actions logs.
4. Run cleanup in dry-run mode, inspect the selected versions and protected references, then exercise deletion with disposable PR image generations.
5. Close a test PR and confirm its image versions disappear. Verify the daily policy preserves the configured generation counts and referenced manifests.

Local tests and builds do not demonstrate GHCR permissions, actual push/deletion, cache restoration across Actions runners, or k3s pull/deployment acceptance. These require the live checks above.

References: [GitHub package deletion](https://docs.github.com/en/packages/learn-github-packages/deleting-and-restoring-a-package), [Docker Actions caches](https://docs.docker.com/build/ci/github-actions/cache/), [GitHub concurrency queues](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency).
