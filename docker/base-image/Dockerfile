##############################
## Build Rust backend
##############################
FROM rust:1.94-trixie AS rust-builder
WORKDIR /wildcat

RUN apt-get update && apt-get install --quiet --no-install-recommends --no-install-suggests --yes \
    protobuf-compiler git

RUN cargo install cargo-chef

ARG CARGO_NET_GIT_FETCH_WITH_CLI=true
ENV CARGO_NET_GIT_FETCH_WITH_CLI=${CARGO_NET_GIT_FETCH_WITH_CLI}

COPY recipe.json recipe.json
COPY --chmod=0755 scripts/cargo_git_auth.sh /scripts/cargo_git_auth.sh
COPY bcr-common bcr-common

RUN --mount=type=secret,id=gh_token,required=false \
    --mount=type=ssh \
    /scripts/cargo_git_auth.sh chef cook --release

COPY . .

RUN --mount=type=secret,id=gh_token,required=false \
    --mount=type=ssh \
    /scripts/cargo_git_auth.sh build --release
