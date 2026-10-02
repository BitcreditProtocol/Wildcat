##############################
## Build Rust backend
##############################
FROM rust:1.94-trixie AS rust-builder
WORKDIR /wildcat

RUN apt-get update && apt-get install --quiet --no-install-recommends --no-install-suggests --yes \
    protobuf-compiler git

ARG CARGO_NET_GIT_FETCH_WITH_CLI=true
ENV CARGO_NET_GIT_FETCH_WITH_CLI=${CARGO_NET_GIT_FETCH_WITH_CLI}

COPY . .

RUN cargo build --release
