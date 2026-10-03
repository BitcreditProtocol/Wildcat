#!/usr/bin/env bash
# Integration lens (STATIC): every internal caller of an admin endpoint that the
# branch moved to an admin-only listener must have a config key naming that listener,
# documented env vars must match each main.rs's config::Environment wiring, and the
# admin-listener doc must list every split service.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
C="${ROOT}/crates"
DOC="${ROOT}/docs/admin-listener.md"
FAILS=()

echo "== 1. core-service -> treasury (fees_store_proofs on every swap)"
grep -n 'fees_store_proofs' "${C}/bcr-wdc-core-service/src/clients.rs"
echo "core-service config.rs url keys:"
grep -nE '_url\s*:' "${C}/bcr-wdc-core-service/src/config.rs"
grep -qE 'treasury_admin_url' "${C}/bcr-wdc-core-service/src/config.rs" \
  || FAILS+=("core-service calls treasury admin endpoint fees_store_proofs but its config has only treasury_url (no treasury admin url)")

echo "== 2. wallet-aggregator -> treasury (try_htlc for HTLC swap inputs)"
grep -n 'try_htlc' "${C}/bcr-wdc-wallet-aggregator/src/web.rs"
echo "wallet-aggregator lib.rs url keys:"
grep -nE '_url\s*:' "${C}/bcr-wdc-wallet-aggregator/src/lib.rs"
echo "shipped example.config.toml:"
grep -n 'treasury' "${C}/bcr-wdc-wallet-aggregator/example.config.toml"
grep -qE 'treasury_admin|treasury_client_admin' "${C}/bcr-wdc-wallet-aggregator/src/lib.rs" \
  || FAILS+=("wallet-aggregator calls treasury admin endpoint try_htlc but its config has only treasury_client_url (no treasury admin url)")

echo "== 3. documented env vars vs config::Environment wiring (config 0.15: prefix separator defaults to separator)"
for pair in core-service:CORE_SERVICE mint-service:MINT_SERVICE quote-service:QUOTE_SERVICE treasury-service:TREASURY_SERVICE; do
  svc=${pair%%:*}; pre=${pair##*:}
  main="${C}/bcr-wdc-${svc}/src/main.rs"
  wiring=$(grep -c "with_prefix(\"${pre}\").separator(\"__\")" "${main}")
  msg=$(grep -o "env ${pre}__ADMIN_BIND_ADDRESS" "${main}")
  echo "  ${svc}: Environment(${pre}, sep __) x${wiring}; panic names '${msg}'"
  [ "${wiring}" -ge 1 ] && [ -n "${msg}" ] || FAILS+=("${svc}: ${pre}__ADMIN_BIND_ADDRESS does not match its Environment wiring")
done
grep -n 'prefix_separator\|with_prefix' "${C}/bcr-wdc-admin-aggregator/src/main.rs"

echo "== 4. docs/admin-listener.md lists every split service"
for v in CORE_SERVICE MINT_SERVICE QUOTE_SERVICE TREASURY_SERVICE; do
  if grep -q "\`${v}__ADMIN_BIND_ADDRESS\`" "${DOC}"; then echo "  ${v}: documented"; else
    echo "  ${v}: MISSING from table"; FAILS+=("docs/admin-listener.md table lacks ${v}__ADMIN_BIND_ADDRESS"); fi
done
grep -n 'Treasury gains' "${DOC}"
grep -nE 'treasury_url|try_htlc|wallet-aggregator' "${DOC}"

echo "== 5. Dockerfiles expose an admin port for each split service"
for svc in core-service mint-service quote-service treasury-service; do
  printf '  %s: ' "${svc}"; grep -c 'EXPOSE 3339' "${ROOT}/docker/${svc}/Dockerfile"
done

if [ ${#FAILS[@]} -gt 0 ]; then
  printf 'FAIL: %s\n' "${FAILS[@]}"
  exit 1
fi
echo PASS
