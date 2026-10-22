#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y --no-install-recommends build-essential pkg-config libssl-dev docker-compose-v2 ca-certificates zstd
# The DinD socket is shared only with this Pod and accessible by the runner group.
for attempt in $(seq 1 90); do
  if [[ -S /var/run/docker/docker.sock ]]; then
    chown root:1001 /var/run/docker/docker.sock
    chmod 660 /var/run/docker/docker.sock
    if docker info >/dev/null 2>&1; then
      exec setpriv --reuid=1001 --regid=1001 --init-groups \
        env HOME=/home/runner USER=runner LOGNAME=runner /bin/bash /runner-start/start.sh
    fi
  fi
  sleep 2
done
echo 'Docker daemon did not become ready' >&2
exit 1
