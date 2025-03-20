##############################
## Build Rust backend
##############################
FROM rust:1.85-bookworm@sha256:caa4a0e7bd1fe2e648caf3d904bc54c3bfcae9e74b4df2eb9ebe558c9e9e88c5 AS rust-builder

WORKDIR /swap-service

# start - build dependency cache
COPY Cargo.toml Cargo.lock ./

RUN mkdir -p ./crates/bcr-wdc-key-service/src
RUN mkdir -p ./crates/bcr-wdc-key-client/src
RUN mkdir -p ./crates/bcr-wdc-quote-service/src
RUN mkdir -p ./crates/bcr-wdc-swap-service/src
RUN mkdir -p ./crates/bcr-wdc-treasury-service/src
RUN mkdir -p ./crates/bcr-wdc-treasury-client/src
RUN mkdir -p ./crates/bcr-wdc-keys/src
RUN mkdir -p ./crates/bcr-wdc-webapi/src

COPY crates/bcr-wdc-key-service/Cargo.toml ./crates/bcr-wdc-key-service/
COPY crates/bcr-wdc-key-client/Cargo.toml ./crates/bcr-wdc-key-client/
COPY crates/bcr-wdc-quote-service/Cargo.toml ./crates/bcr-wdc-quote-service/
COPY crates/bcr-wdc-swap-service/Cargo.toml ./crates/bcr-wdc-swap-service/
COPY crates/bcr-wdc-treasury-service/Cargo.toml ./crates/bcr-wdc-treasury-service/
COPY crates/bcr-wdc-treasury-client/Cargo.toml ./crates/bcr-wdc-treasury-client/
COPY crates/bcr-wdc-keys/Cargo.toml ./crates/bcr-wdc-keys/
COPY crates/bcr-wdc-webapi/Cargo.toml ./crates/bcr-wdc-webapi/

RUN echo 'fn main() { panic!("Dummy Image Called!")}' > ./crates/bcr-wdc-key-service/src/main.rs
RUN echo 'fn main() { panic!("Dummy Image Called!")}' > ./crates/bcr-wdc-key-client/src/main.rs
RUN echo 'fn main() { panic!("Dummy Image Called!")}' > ./crates/bcr-wdc-quote-service/src/main.rs
RUN echo 'fn main() { panic!("Dummy Image Called!")}' > ./crates/bcr-wdc-swap-service/src/main.rs
RUN echo 'fn main() { panic!("Dummy Image Called!")}' > ./crates/bcr-wdc-treasury-service/src/main.rs
RUN echo 'fn main() { panic!("Dummy Image Called!")}' > ./crates/bcr-wdc-treasury-client/src/main.rs
RUN echo 'fn main() { panic!("Dummy Image Called!")}' > ./crates/bcr-wdc-keys/src/main.rs
RUN echo 'fn main() { panic!("Dummy Image Called!")}' > ./crates/bcr-wdc-webapi/src/main.rs

# Build dependencies (without compiling main source files)
RUN --mount=type=cache,target=/usr/local/cargo/registry cargo build --release -p bcr-wdc-swap-service
# end - build dependency cache

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  # update timestamps to force a new build
  touch -a -m ./crates/bcr-wdc-swap-service/src/main.rs \
  && cargo build --release -p bcr-wdc-swap-service

##############################
## Create image
##############################
FROM debian:bookworm-20250224-slim@sha256:12c396bd585df7ec21d5679bb6a83d4878bc4415ce926c9e5ea6426d23c60bdc
RUN apt-get update \
  && apt-get install --quiet --no-install-recommends --no-install-suggests --yes openssl curl \
  && apt-get clean \
  && rm --recursive --force /var/lib/apt/lists/*

WORKDIR /swap-service

COPY --from=rust-builder /swap-service/target/release/bcr-wdc-swap-service .

# Expose web server port
EXPOSE 3338

CMD ["/swap-service/bcr-wdc-swap-service"]
