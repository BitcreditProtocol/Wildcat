#!/usr/bin/env bash
set -Eeuo pipefail

tmp="$(mktemp -d)"
export GIT_CONFIG_GLOBAL="$tmp/.gitconfig"

# cleanup git auth config on exit
cleanup() { rm -rf "$tmp" || true; }
trap 'rc=$?; cleanup; exit $rc' EXIT

# git auth
if [[ -f /run/secrets/gh_token ]]; then
  GH_TOKEN="$(cat /run/secrets/gh_token)"
  git config --global --add url."https://github.com/".insteadOf "ssh://git@github.com/"
  git config --global --add url."https://github.com/".insteadOf "git@github.com:"
  git config --global --add url."https://github.com/".insteadOf "git://github.com/"
  git config --global url."https://x-access-token:${GH_TOKEN}@github.com/".insteadOf "https://github.com/"
  unset GH_TOKEN
elif [[ -n "${SSH_AUTH_SOCK:-}" ]]; then
  mkdir -p -m 0700 "$tmp/.ssh"
  ssh-keyscan -t rsa,ecdsa,ed25519 github.com >> "$tmp/.ssh/known_hosts"
  chmod 0644 "$tmp/.ssh/known_hosts"
  git config --global core.sshCommand \
    "ssh -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$tmp/.ssh/known_hosts"
  git config --global url."git@github.com:".insteadOf "https://github.com/"
fi

# run cargo with passed args
cargo "$@"