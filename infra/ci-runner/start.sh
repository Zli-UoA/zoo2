#!/bin/bash
set -euo pipefail
cd /home/runner
child=''
stopping=0
api_token() {
  # Read the credential on each request so Secret rotation is picked up.
  { printf 'header = "Authorization: Bearer %s"\n' "$(cat /runner-auth/token)"; } |
    curl --config - --fail --silent --show-error \
      --connect-timeout 10 --max-time 30 --request POST \
      --header 'Accept: application/vnd.github+json' \
      "https://api.github.com/repos/Zli-UoA/zoo2/actions/runners/$1" |
    jq -er '.token'
}
cleanup() {
  local status=$?
  trap - EXIT TERM INT
  if [[ -f .runner ]]; then
    if token=$(api_token remove-token); then
      ./config.sh remove --unattended --token "$token" ||
        echo 'Runner removal failed; check GitHub runner settings.' >&2
      unset token
    else
      echo 'Could not obtain removal token; check GitHub runner settings.' >&2
    fi
  fi
  exit "$status"
}
stop() {
  stopping=1
  if [[ -n "$child" ]]; then kill -TERM "$child" 2>/dev/null || true; fi
}
trap cleanup EXIT
trap stop TERM INT
if [[ ! -f .runner ]]; then
  token=$(api_token registration-token)
  ./config.sh --unattended \
    --url https://github.com/Zli-UoA/zoo2 \
    --token "$token" --name "zoo2-${POD_UID}" --replace \
    --labels "${RUNNER_LABELS:-zoo2}" --work /home/runner/_work
  unset token
fi
if (( stopping )); then exit 0; fi
./run.sh &
child=$!
# A trapped signal interrupts wait: wait again until the child really exits.
set +e
wait "$child"
status=$?
if (( stopping )); then
  wait "$child"
  status=$?
fi
exit "$status"
