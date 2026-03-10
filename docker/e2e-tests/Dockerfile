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

WORKDIR /e2e-tests
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-e2e-tests .

# Expose web server port
EXPOSE 3338

ENTRYPOINT ["/e2e-tests/bcr-wdc-e2e-tests"]
