#!/usr/bin/env bash
# Boots the demo from examples/admin_listener_demo.rs and confirms live, over HTTP,
# that the admin router is unreachable from the public listener's address and the
# admin listener itself is reachable.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

WEB_ADDR="127.0.0.1:18338"
ADMIN_ADDR="127.0.0.1:18339"
ADMIN_SIGN_PATH="/admin/keys/sign"

cd "${CRATE_DIR}"

# `cargo run` keeps the compiled binary as its own child process, not its own
# exec'd replacement, so killing only cargo's PID leaves that child running and
# still bound to the port for the next invocation (`setsid` is not a fix here:
# it silently does nothing when the caller is already a process group leader,
# which a backgrounded job in a script often is). Kill the child by its actual
# parent PID instead, which needs no session or process-group cooperation.
cargo run --quiet --example admin_listener_demo --features test-utils &
DEMO_PID=$!

cleanup() {
    pkill -P "${DEMO_PID}" >/dev/null 2>&1
    kill "${DEMO_PID}" >/dev/null 2>&1
    wait "${DEMO_PID}" 2>/dev/null
}
trap cleanup EXIT

# cargo serializes on the target directory's build lock, which the fleet's full
# check and other phases' builds can hold for minutes under a shared build slot
# (see CLAUDE.md: "each one waits its turn"); a compiled binary itself starts in
# a few seconds, so almost all of this budget is headroom for that queueing, not
# a tolerance for a slow server.
READY=0
for _ in $(seq 1 300); do
    if curl --silent --output /dev/null --fail "http://${WEB_ADDR}/health"; then
        READY=1
        break
    fi
    sleep 1
done

if [ "${READY}" -ne 1 ]; then
    echo "FAIL: web listener never came up on ${WEB_ADDR}"
    exit 1
fi

FAIL=0

admin_on_web=$(curl --silent --output /dev/null --write-out '%{http_code}' -X POST "http://${WEB_ADDR}${ADMIN_SIGN_PATH}")
if [ "${admin_on_web}" = "404" ]; then
    echo "PASS: admin SIGN path on the public listener answers 404 (got ${admin_on_web})"
else
    echo "FAIL: admin SIGN path on the public listener answered ${admin_on_web}, expected 404"
    FAIL=1
fi

admin_on_admin=$(curl --silent --output /dev/null --write-out '%{http_code}' -X POST "http://${ADMIN_ADDR}${ADMIN_SIGN_PATH}")
if [ "${admin_on_admin}" != "404" ] && [ -n "${admin_on_admin}" ]; then
    echo "PASS: admin SIGN path on the admin listener is reachable (got ${admin_on_admin}, not 404)"
else
    echo "FAIL: admin listener at ${ADMIN_ADDR} did not answer the admin SIGN path (got '${admin_on_admin}')"
    FAIL=1
fi

web_health_on_admin=$(curl --silent --output /dev/null --write-out '%{http_code}' "http://${ADMIN_ADDR}/health")
if [ "${web_health_on_admin}" = "200" ]; then
    echo "FAIL: public /health is reachable on the admin listener (got ${web_health_on_admin})"
    FAIL=1
else
    echo "PASS: public /health is not reachable on the admin listener (got ${web_health_on_admin})"
fi

exit "${FAIL}"
