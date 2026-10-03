#!/usr/bin/env bash
# Request lens (STATIC): admin-aggregator serves admin operations on one public
# listener, so it must read a secret and wrap its router in auth. Fails when its
# main.rs/lib.rs read no secret and never call an auth layer.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="${ROOT}/crates/bcr-wdc-admin-aggregator/src"
echo "MainConfig fields:"
sed -n '/struct MainConfig/,/^}/p' "${SRC}/main.rs"
HITS=$(grep -niE 'require_api_key|api_key|secret|bearer|authoriz|auth::' "${SRC}/main.rs" "${SRC}/lib.rs")
echo "auth/secret references in main.rs+lib.rs: ${HITS:-none}"
echo "require_api_key callers outside bcr-wdc-utils:"
grep -rln 'require_api_key' "${ROOT}/crates" --include=*.rs | grep -v 'bcr-wdc-utils' || echo "  none"
[ -n "${HITS}" ] || { echo "FAIL: admin-aggregator reads no secret and applies no auth layer"; exit 1; }
