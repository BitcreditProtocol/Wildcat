##############################
## Build Rust backend
##############################
FROM rust:1.89-bookworm AS rust-builder
WORKDIR /wildcat

RUN apt-get update && apt-get install --quiet --no-install-recommends --no-install-suggests --yes \
    protobuf-compiler git

RUN cargo install cargo-chef

ARG CARGO_NET_GIT_FETCH_WITH_CLI=true
ENV CARGO_NET_GIT_FETCH_WITH_CLI=${CARGO_NET_GIT_FETCH_WITH_CLI}

COPY recipe.json recipe.json

RUN --mount=type=secret,id=gh_token,required=false \
    --mount=type=ssh <<'BASH'

if [ -f /run/secrets/gh_token ]; then
  GH_TOKEN="$(cat /run/secrets/gh_token)"
  git config --global --add url."https://github.com/".insteadof "ssh://git@github.com/"
  git config --global --add url."https://github.com/".insteadof "git@github.com:"
  git config --global --add url."https://github.com/".insteadof "git://github.com/"
  git config --global url."https://x-access-token:${GH_TOKEN}@github.com/".insteadof "https://github.com/"
elif [ -n "${SSH_AUTH_SOCK:-}" ]; then
  mkdir -p -m 0700 /root/.ssh
  ssh-keyscan -t rsa,ecdsa,ed25519 github.com >> /root/.ssh/known_hosts
  chmod 0644 /root/.ssh/known_hosts
  git config --global core.sshCommand "ssh -o StrictHostKeyChecking=yes -o UserKnownHostsFile=/root/.ssh/known_hosts"
  git config --global url."git@github.com:".insteadof "https://github.com/"
fi

cargo chef cook --release

rm -f /root/.gitconfig /root/.ssh/known_hosts || true
BASH

COPY . .

RUN --mount=type=secret,id=gh_token,required=false \
    --mount=type=ssh <<'BASH'

if [ -f /run/secrets/gh_token ]; then
  GH_TOKEN="$(cat /run/secrets/gh_token)"
  git config --global --add url."https://github.com/".insteadof "ssh://git@github.com/"
  git config --global --add url."https://github.com/".insteadof "git@github.com:"
  git config --global --add url."https://github.com/".insteadof "git://github.com/"
  git config --global url."https://x-access-token:${GH_TOKEN}@github.com/".insteadof "https://github.com/"
elif [ -n "${SSH_AUTH_SOCK:-}" ]; then
  mkdir -p -m 0700 /root/.ssh
  ssh-keyscan -t rsa,ecdsa,ed25519 github.com >> /root/.ssh/known_hosts
  chmod 0644 /root/.ssh/known_hosts
  git config --global core.sshCommand "ssh -o StrictHostKeyChecking=yes -o UserKnownHostsFile=/root/.ssh/known_hosts"
  git config --global url."git@github.com:".insteadof "https://github.com/"
fi

cargo build --release

rm -f /root/.gitconfig /root/.ssh/known_hosts || true
BASH