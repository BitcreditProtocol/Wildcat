##############################
## Build Rust backend
##############################
FROM rust:1.89-bookworm AS rust-builder
WORKDIR /wildcat

RUN apt-get update && apt-get install --quiet --no-install-recommends --no-install-suggests --yes protobuf-compiler git
RUN cargo install cargo-chef

ARG CARGO_NET_GIT_FETCH_WITH_CLI=true
ENV CARGO_NET_GIT_FETCH_WITH_CLI=${CARGO_NET_GIT_FETCH_WITH_CLI}

RUN --mount=type=secret,id=gh_token,required=false \
    sh -eu -c 'if [ -f /run/secrets/gh_token ]; then \
      GH_TOKEN="$(cat /run/secrets/gh_token)"; \
      git config --global url."https://x-access-token:${GH_TOKEN}@github.com/".insteadOf "https://github.com/"; \
    else \
      ssh-keyscan github.com > /etc/ssh/ssh_known_hosts; \
      git config --global url."git@github.com:".insteadOf "https://github.com/"; \
    fi'

COPY recipe.json recipe.json
RUN --mount=type=ssh cargo chef cook --release


COPY . .
RUN --mount=type=ssh cargo build --release
