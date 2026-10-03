#!/usr/bin/env bash
# Request lens: "confirm the ingress never exposes them". For each shipped service
# Dockerfile, builds a throwaway image carrying exactly that Dockerfile's EXPOSE lines
# (sleep + its libs from this host as the payload), runs it with `docker run -P` and
# reads `docker port`. Fails when the admin port (3339, per the Dockerfile comments
# and docs/admin-listener.md) is published on the host. Falls back to a STATIC listing
# when docker is unusable.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ADMIN_PORT=3339
FAIL=0
if ! docker info >/dev/null 2>&1; then
  echo "docker unavailable: STATIC listing only"
  grep -Hn '^EXPOSE' "${ROOT}"/docker/*/Dockerfile
  grep -qs "^EXPOSE ${ADMIN_PORT}" "${ROOT}"/docker/*/Dockerfile && exit 1
  exit 0
fi
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
mkdir -p "${WORK}/rf"
cp /usr/bin/sleep "${WORK}/rf/"
for l in $(ldd /usr/bin/sleep | grep -o '/[^ ]*'); do
  mkdir -p "${WORK}/rf$(dirname "$l")"; cp -L "$l" "${WORK}/rf$l"
done
tar -C "${WORK}/rf" -cf "${WORK}/rf.tar" .
for svc in core-service mint-service quote-service treasury-service admin-aggregator wallet-aggregator; do
  df="${ROOT}/docker/${svc}/Dockerfile"
  [ -f "$df" ] || continue
  final=$(awk '/^FROM /{n=NR} END{print n}' "$df")
  changes=()
  while read -r port; do changes+=(--change "EXPOSE ${port}"); done < <(awk -v s="$final" 'NR>s && /^EXPOSE /{print $2}' "$df")
  img="probe-request-expose-${svc}:test"
  docker import "${changes[@]}" --change 'ENTRYPOINT ["/sleep","60"]' "${WORK}/rf.tar" "$img" >/dev/null
  cid=$(docker run -d --rm -P "$img")
  published=$(docker port "$cid")
  docker rm -f "$cid" >/dev/null 2>&1
  docker rmi "$img" >/dev/null 2>&1
  echo "${svc}: EXPOSE $(awk -v s="$final" 'NR>s && /^EXPOSE /{printf "%s ", $2}' "$df")-> docker run -P publishes:"
  echo "${published:-  (nothing)}" | sed 's/^/  /'
  if echo "$published" | grep -q "^${ADMIN_PORT}/tcp"; then
    echo "FAIL: ${svc} admin port ${ADMIN_PORT} is published on the host by docker run -P"
    FAIL=1
  fi
done
echo "ingress/compose/envoy config shipped in repo:"
git -C "${ROOT}" ls-files | grep -iE 'compose|envoy|ingress|nginx|traefik|helm|k8s' || echo "  none"
exit "${FAIL}"
