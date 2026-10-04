#!/usr/bin/env bash
# Every env var docs/admin-listener.md tells a deployer to set must actually set
# its key in the real binary: start it with config.toml lacking that key and the
# documented env var set, and check startup gets past config parsing for it.
# Binaries are built by probes/run.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${CARGO_TARGET_DIR:-${ROOT}/target}/debug"
DOC="${ROOT}/docs/admin-listener.md"
FAILS=()
MNEMONIC="abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"

core_toml() { cat <<EOF
bind_address = "127.0.0.1:0"
$([ "$1" = admin_bind_address ] || echo 'admin_bind_address = "127.0.0.1:0"')
log_level = "INFO"
[appcfg]
clowder_url = "http://127.0.0.1:9"
clowder_rest_url = "http://127.0.0.1:9"
$([ "$1" = treasury_admin_url ] || echo 'treasury_admin_url = "http://127.0.0.1:9"')
starting_derivation_path = "m/0'"
max_expiry_sec = 3600
minimum_keyset_fees_ppk = 0
cache_expiry_sec = 60
settle_window_sec = 60
[appcfg.repository]
connection = "ws://127.0.0.1:9"
namespace = "n"
database = "d"
[appcfg.repository_new]
connection = "postgres://u:p@127.0.0.1:9/d"
max_connections = 1
EOF
}

quote_toml() { cat <<EOF
bind_address = "127.0.0.1:0"
$([ "$1" = admin_bind_address ] || echo 'admin_bind_address = "127.0.0.1:0"')
log_level = "INFO"
[appcfg]
core_url = "http://127.0.0.1:9"
$([ "$1" = core_admin_url ] || echo 'core_admin_url = "http://127.0.0.1:9"')
$([ "$1" = treasury_admin_url ] || echo 'treasury_admin_url = "http://127.0.0.1:9"')
ebill_url = "http://127.0.0.1:9"
clowder_url = "http://127.0.0.1:9"
monitor_interval_seconds = 5
[appcfg.quotes]
connection = "ws://127.0.0.1:9"
namespace = "n"
database = "d"
EOF
}

wallet_toml() { cat <<EOF
bind_address = "127.0.0.1:0"
log_level = "INFO"
[appcfg]
core_client_url = "http://127.0.0.1:9"
$([ "$1" = treasury_admin_client_url ] || echo 'treasury_admin_client_url = "http://127.0.0.1:9"')
clwdr_rest_url = "http://127.0.0.1:9"
EOF
}

# check <binary> <toml-fn> <key> <ENV_VAR> <value>
check() {
  local bin="$1" tomlfn="$2" key="$3" var="$4" val="$5" dir out
  [ "${CONTROL:-}" = 1 ] || grep -q "${var}" "${DOC}" || { echo "  (${var} not documented, skipped)"; return; }
  dir="$(mktemp -d)"
  "${tomlfn}" "${key}" > "${dir}/config.toml"
  out="$(cd "${dir}" && env -i PATH="${PATH}" CORE_SERVICE_MNEMONIC="${MNEMONIC}" "${var}=${val}" \
    timeout 8 "${BIN}/${bin}" 2>&1 >/dev/null)"
  local st=$?
  rm -rf "${dir}"
  if grep -qE "missing (configuration )?(field|required setting) [\"\`]([a-z_]+\.)?${key}[\"\`]" <<<"${out}"; then
    echo "  ${var}: IGNORED -> $(grep -m1 -oE "missing (configuration )?(field|required setting) [\"\`]([a-z_]+\.)?${key}[\"\`]" <<<"${out}")"
    [ "${CONTROL:-}" = 1 ] || FAILS+=("${bin}: documented ${var} does not set ${key}")
  else
    echo "  ${var}: honoured (exit ${st}; next startup step: $(grep -m1 -oE "panicked at [^:]+:[0-9]+|Failed to [a-z ]+|Error[^,]{0,60}" <<<"${out}" || echo 'still running'))"
  fi
}

for b in bcr-wdc-core-service bcr-wdc-quote-service bcr-wdc-wallet-aggregator; do
  [ -x "${BIN}/${b}" ] || { echo "FAIL: ${BIN}/${b} not built"; exit 1; }
done

echo "== core-service"
check bcr-wdc-core-service core_toml treasury_admin_url CORE_SERVICE__APPCFG__TREASURY_ADMIN_URL http://127.0.0.1:9
check bcr-wdc-core-service core_toml admin_bind_address CORE_SERVICE__ADMIN_BIND_ADDRESS 127.0.0.1:0
echo "== quote-service"
check bcr-wdc-quote-service quote_toml core_admin_url QUOTE_SERVICE__CORE_ADMIN_URL http://127.0.0.1:9
check bcr-wdc-quote-service quote_toml treasury_admin_url QUOTE_SERVICE__TREASURY_ADMIN_URL http://127.0.0.1:9
check bcr-wdc-quote-service quote_toml admin_bind_address QUOTE_SERVICE__ADMIN_BIND_ADDRESS 127.0.0.1:0
echo "  control (undocumented, nested under APPCFG):"
CONTROL=1 check bcr-wdc-quote-service quote_toml core_admin_url QUOTE_SERVICE__APPCFG__CORE_ADMIN_URL http://127.0.0.1:9
echo "== wallet-aggregator"
check bcr-wdc-wallet-aggregator wallet_toml treasury_admin_client_url WALLET_AGGREGATOR__APPCFG__TREASURY_ADMIN_CLIENT_URL http://127.0.0.1:9

if [ ${#FAILS[@]} -gt 0 ]; then
  printf 'FAIL: %s\n' "${FAILS[@]}"
  exit 1
fi
echo PASS
