##############################
## Build Rust backend
##############################
FROM rust:1.85-bookworm AS rust-builder

WORKDIR /wildcat

RUN cargo install cargo-chef

COPY recipe.json recipe.json
RUN cargo chef cook --release


COPY . .
RUN cargo build --release
