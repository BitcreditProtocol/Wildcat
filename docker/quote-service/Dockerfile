##############################
## Build Rust backend
##############################
FROM rust:1.85-bookworm AS rust-builder

WORKDIR /quote-service
COPY . .
RUN cargo build --release -p bcr-wdc-quote-service

##############################
## Create image
##############################
FROM debian:bookworm-slim
RUN apt-get update && apt install -y openssl

WORKDIR /quote-service

COPY --from=rust-builder /quote-service/target/release/bcr-wdc-quote-service .

# Expose web server port
EXPOSE 3338

CMD ["/quote-service/bcr-wdc-quote-service"]
