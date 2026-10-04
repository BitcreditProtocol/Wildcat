#!/usr/bin/env bash
# The core-service admin-isolation check must build the demo example in the
# foreground before it ever backgrounds or polls it, so a compile error fails
# loudly and the readiness wait is never spent on compile time. Checks the
# script's own structure, then proves it from an actually cold build: delete
# the compiled example binary and time the whole script.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="${ROOT}/crates/bcr-wdc-core-service/scripts/check_admin_isolation.sh"
FAIL=0

if ! grep -q 'cargo build .*admin_listener_demo' "${SCRIPT}"; then
  echo "FAIL: script never runs 'cargo build' for the demo example before use"
  FAIL=1
else
  echo "PASS: script builds the demo example with 'cargo build'"
fi

if grep -qE 'cargo run .*admin_listener_demo' "${SCRIPT}"; then
  echo "FAIL: script still starts the demo with 'cargo run', which hides a compile error inside the readiness wait"
  FAIL=1
else
  echo "PASS: script does not start the demo with 'cargo run'"
fi

BUILD_LINE="$(grep -n 'cargo build' "${SCRIPT}" | head -1 | cut -d: -f1)"
BG_LINE="$(grep -n '&$' "${SCRIPT}" | head -1 | cut -d: -f1)"
if [ -z "${BUILD_LINE}" ] || [ -z "${BG_LINE}" ] || [ "${BUILD_LINE}" -ge "${BG_LINE}" ]; then
  echo "FAIL: the build does not run before the demo is backgrounded (build at line ${BUILD_LINE:-?}, background at line ${BG_LINE:-?})"
  FAIL=1
else
  echo "PASS: the build (line ${BUILD_LINE}) runs before the demo is backgrounded (line ${BG_LINE})"
fi

CRATE_DIR="${ROOT}/crates/bcr-wdc-core-service"
DEMO_BIN="${ROOT}/target/debug/examples/admin_listener_demo"
rm -f "${DEMO_BIN}"
if [ -e "${DEMO_BIN}" ]; then
  echo "FAIL: could not remove the compiled demo binary to force a cold build"
  FAIL=1
fi

START=$(date +%s)
if bash "${SCRIPT}"; then
  END=$(date +%s)
  echo "PASS: check passed from a cold build in $((END - START))s"
else
  echo "FAIL: check did not pass from a cold build"
  FAIL=1
fi

exit "${FAIL}"
