# CI runner selection

Both CI workflows run a small selector on GitHub-hosted Ubuntu. It checks the
repository runner API and selects an online, idle runner with all required labels.
Missing credentials, API errors, offline/busy runners, and pull requests use
GitHub-hosted Ubuntu. Manual dispatch supports `runner: github` to bypass selection.

- Frontend check/unit-test: `self-hosted, Linux, X64, zoo2` when available.
- Backend build/lint/unit-test: additionally requires `zoo2-rust`, which must only
  be assigned after provisioning a C compiler/linker and Rust build prerequisites.
  The namespace runner is provisioned with these prerequisites.
- Docker Compose jobs (backend e2e and frontend container/proxy): additionally
  requires `zoo2-docker`. The namespace runner uses a privileged DinD sidecar,
  a shared Unix socket and workspace, with Docker Compose installed.

`RUNNER_STATUS_TOKEN` is a repository Actions secret with permission to list
repository runners (fine-grained PAT: repository Administration read). It is used
only by the checkout-free selector, never by test steps. Use a dedicated read-only
credential for this secret. Never run untrusted code on the persistent runner.

Selection is a point-in-time availability check, not a reservation. Multiple jobs
may queue for the same runner. If it goes offline after selection, GitHub does not
automatically move the queued job to a hosted runner: cancel that run and dispatch
with `runner: github`. For validation, `runner: self-hosted` explicitly selects the
namespace runner without checking availability (it may queue if unavailable). A failing test remains a CI failure, not an automatic retry
on a different platform. Job timeouts limit running jobs, not queue time.
