ARG WILDCAT_BASE_IMAGE=wildcat/base-image
FROM ${WILDCAT_BASE_IMAGE} AS rust-builder

##############################
## Create image
##############################
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install --quiet --no-install-recommends --no-install-suggests --yes openssl curl ca-certificates \
    && apt-get clean \
    && rm --recursive --force /var/lib/apt/lists/*

WORKDIR /ebpp
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-ebpp .

# Expose gRPC port
EXPOSE 9090

ENTRYPOINT ["/ebpp/bcr-wdc-ebpp"]
