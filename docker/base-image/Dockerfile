##############################
## Build Rust backend
##############################
FROM rust:1.94-trixie AS chef
WORKDIR /wildcat

RUN apt-get update && apt-get install --quiet --no-install-recommends --no-install-suggests --yes \
    protobuf-compiler git

RUN cargo install cargo-chef

ARG CARGO_NET_GIT_FETCH_WITH_CLI=true
ENV CARGO_NET_GIT_FETCH_WITH_CLI=${CARGO_NET_GIT_FETCH_WITH_CLI}

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS rust-builder
COPY --from=planner /wildcat/recipe.json recipe.json
COPY --chmod=0755 scripts/cargo_git_auth.sh /scripts/cargo_git_auth.sh

RUN --mount=type=secret,id=gh_token,required=false \
    --mount=type=ssh \
    /scripts/cargo_git_auth.sh chef cook --release

COPY . .

RUN --mount=type=secret,id=gh_token,required=false \
    --mount=type=ssh \
    /scripts/cargo_git_auth.sh build --release
