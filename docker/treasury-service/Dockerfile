##############################
## Build Rust backend
##############################
FROM rust:1.85-bookworm@sha256:caa4a0e7bd1fe2e648caf3d904bc54c3bfcae9e74b4df2eb9ebe558c9e9e88c5 AS rust-builder

WORKDIR /treasury-service
COPY . .
RUN cargo build --release -p bcr-wdc-treasury-service

##############################
## Create image
##############################
FROM debian:bookworm-20250224-slim@sha256:12c396bd585df7ec21d5679bb6a83d4878bc4415ce926c9e5ea6426d23c60bdc
RUN apt-get update && apt install -y openssl

WORKDIR /treasury-service

COPY --from=rust-builder /treasury-service/target/release/bcr-wdc-treasury-service .

# Expose web server port
EXPOSE 3338

CMD ["/treasury-service/bcr-wdc-treasury-service"]
