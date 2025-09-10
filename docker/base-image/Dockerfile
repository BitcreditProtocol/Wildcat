##############################
## Build Rust backend
##############################
FROM rust:1.89-bookworm AS rust-builder

WORKDIR /wildcat

RUN apt-get update && apt-get install --quiet --no-install-recommends --no-install-suggests --yes protobuf-compiler
RUN cargo install cargo-chef

COPY recipe.json recipe.json
RUN cargo chef cook --release


COPY . .
RUN cargo build --release
