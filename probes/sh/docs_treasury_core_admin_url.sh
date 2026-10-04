#!/usr/bin/env bash
# docs/admin-listener.md must document treasury-service's core_admin_url setting
# (it reads one, per crates/bcr-wdc-treasury-service/src/config.rs) and must no
# longer tell a deployer that treasury's callers still point at core's public
# address, since they were moved to the admin client in src/lib.rs.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DOC="${ROOT}/docs/admin-listener.md"
FAIL=0

if grep -q 'TREASURY_SERVICE__APPCFG__CORE_ADMIN_URL' "${DOC}"; then
  echo "PASS: doc names treasury-service's core_admin_url env var"
else
  echo "FAIL: doc never names TREASURY_SERVICE__APPCFG__CORE_ADMIN_URL"
  FAIL=1
fi

if grep -qE 'core_admin_url' "${DOC}" && grep -B2 -A2 'TREASURY_SERVICE__APPCFG__CORE_ADMIN_URL' "${DOC}" | grep -qi 'treasury'; then
  echo "PASS: core_admin_url is documented in a treasury-service context"
else
  echo "FAIL: core_admin_url is not documented next to treasury-service"
  FAIL=1
fi

if grep -q 'Today every caller' "${DOC}"; then
  echo "FAIL: doc still says every caller points at core's public address"
  FAIL=1
else
  echo "PASS: doc no longer claims every caller still points at core's public address"
fi

# The config field really exists, so the doc is not inventing a setting.
if git -C "${ROOT}" show HEAD:crates/bcr-wdc-treasury-service/src/config.rs | grep -q 'pub core_admin_url: ClientUrl'; then
  echo "PASS: treasury-service's config really has core_admin_url"
else
  echo "FAIL: treasury-service's config has no core_admin_url field"
  FAIL=1
fi

exit "${FAIL}"
