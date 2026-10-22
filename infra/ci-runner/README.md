# Kubernetes runner in a configurable namespace

This Deployment runs one persistent GitHub Actions runner and a privileged Docker
in Docker sidecar. No CRDs, ClusterRoles, RoleBindings, hostPath mounts, or Kubernetes
service account token are required. Admission policy must allow privileged DinD.
The runner starts as root to install build prerequisites, then runs jobs as UID 1001.
Jobs can control the privileged Docker daemon: only trusted repository code belongs
on this runner. Pull request workflows select disposable GitHub-hosted runners.

The reference environment has a 3000Mi memory / 2 CPU quota; its other workloads
reserve 500Mi / 250m. Check and adapt these values for your own namespace. This Pod limits runner and Docker memory to 1200Mi each and CPU to
1 and 500m respectively. Only **one** such Pod fits. Do not scale beyond one without
increasing quota or changing the resource allocation. Rust compilation uses one
build job to reduce memory pressure. Docker Compose jobs and Rust builds share the
Pod's resources; they execute sequentially through the single runner.

Create Secret `zoo2-runner-api` with key `token`, containing a GitHub credential able
to register/remove runners for Zli-UoA/zoo2 (repository Administration write for a
fine-grained PAT). This credential is separate from the read-only Actions secret
`RUNNER_STATUS_TOKEN` used by workflow selection. Do not commit either credential.

From the repository root, set `RUNNER_NAMESPACE` locally to your namespace.
The namespace may identify a person; do not commit it or use it as a runner label.
The manifest intentionally omits `metadata.namespace` and is reusable across namespaces.

```bash
read -rp "Runner namespace: " RUNNER_NAMESPACE
kubectl -n "$RUNNER_NAMESPACE" create configmap zoo2-runner-start \
  --from-file=start.sh=infra/ci-runner/start.sh \
  --from-file=bootstrap.sh=infra/ci-runner/bootstrap.sh \
  --dry-run=client -o yaml | kubectl -n "$RUNNER_NAMESPACE" apply -f -
kubectl -n "$RUNNER_NAMESPACE" apply -f infra/ci-runner/deployment.yaml
kubectl -n "$RUNNER_NAMESPACE" logs -f deployment/zoo2-runner -c runner
```

Wait for `Listening for Jobs` and confirm Online in GitHub. Kubernetes readiness
alone does not prove runner registration. Startup installs Ubuntu packages and
therefore needs package-mirror access. The shared workspace and Docker data are
emptyDir volumes and are lost when the Pod is recreated. The Docker socket is a
Pod-local Unix socket; the daemon is not exposed through a Service or host mount.

Update only while idle. Deployment uses Recreate to stay within quota. Scaling
down or updating can cancel running jobs; there is no automatic busy-aware drain.
Normal termination attempts registration removal; forced termination may leave a
stale registration that must be removed in GitHub Settings → Actions → Runners.
The runner includes zstd for Actions cache compression.
Update runner/Docker images regularly; image tags currently track upstream updates.
