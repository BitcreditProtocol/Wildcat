#!/usr/bin/env bash
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
test_tmp="$(mktemp -d)"
export CALL_LOG="$test_tmp/calls.log"
trap 'rm -rf "$test_tmp"' EXIT

ssh-add() {
  printf 'ssh-add\n' >> "$CALL_LOG"
  return "${SSH_ADD_RC:-0}"
}

ssh-keyscan() {
  printf 'ssh-keyscan\n' >> "$CALL_LOG"
  printf 'github.com ssh-ed25519 test-key\n'
}

git() {
  printf 'git %s\n' "$*" >> "$CALL_LOG"
}

cargo() {
  printf 'cargo %s\n' "$*" >> "$CALL_LOG"
}

export -f ssh-add ssh-keyscan git cargo

assert_not_called() {
  if grep -q "^$1" "$CALL_LOG"; then
    printf 'unexpected call in %s case: %s\n' "$2" "$1" >&2
    exit 1
  fi
}

: > "$CALL_LOG"
env -u SSH_AUTH_SOCK SSH_ADD_RC=0 bash "$script_dir/cargo_git_auth.sh" metadata
assert_not_called ssh-add "no socket"
assert_not_called ssh-keyscan "no socket"
assert_not_called git "no socket"

: > "$CALL_LOG"
SSH_AUTH_SOCK="$test_tmp/unusable.sock" SSH_ADD_RC=1 bash "$script_dir/cargo_git_auth.sh" metadata
grep -q '^ssh-add$' "$CALL_LOG"
assert_not_called ssh-keyscan "unusable agent"
assert_not_called git "unusable agent"

: > "$CALL_LOG"
SSH_AUTH_SOCK="$test_tmp/usable.sock" SSH_ADD_RC=0 bash "$script_dir/cargo_git_auth.sh" metadata
grep -q '^ssh-add$' "$CALL_LOG"
grep -q '^ssh-keyscan$' "$CALL_LOG"
grep -q '^git config --global url.git@github.com:.insteadOf https://github.com/$' "$CALL_LOG"

printf 'cargo_git_auth checks passed\n'
