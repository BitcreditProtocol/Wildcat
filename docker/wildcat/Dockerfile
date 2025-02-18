##############################
## Build Rust backend
##############################
FROM rust:1.84.1-bookworm AS rust-builder

WORKDIR /wildcat
COPY . .
RUN cargo build --release -p wildcat

##############################
## Create image
##############################
FROM debian:bookworm-slim
RUN apt-get update && apt install -y openssl

WORKDIR /wildcat

COPY --from=rust-builder /wildcat/target/release/wildcat .

# Expose web server port
EXPOSE 3338

CMD ["/wildcat/wildcat"]
