##############################
## Build Rust backend
##############################
FROM rust:1.84.1-bookworm AS rust-builder

# create a new empty shell project
RUN USER=root cargo new --bin wildcat
WORKDIR /wildcat

# copy manifests and build dependencies
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
RUN cargo build --release
RUN rm src/*.rs

# copy source tree
COPY ./src ./src

RUN cargo build --release

##############################
## Create image
##############################
FROM debian:bookworm-slim
RUN apt-get update && apt install -y openssl

WORKDIR /wildcat

# Copy essential build files
COPY --from=rust-builder /wildcat/target/release/wildcat .
COPY wildcat.toml .

# Create additional directories and set user permissions
RUN mkdir data

ENV ROCKET_ADDRESS=0.0.0.0

# Expose web server port
EXPOSE 3338

CMD ["/wildcat/wildcat"]
