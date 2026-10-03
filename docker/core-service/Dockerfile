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

WORKDIR /core-service
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-core-service .
COPY --from=rust-builder /wildcat/target/release/core-migrate ./migrate

# Expose web server port
EXPOSE 3338
# Admin port (bind_address for the admin router) must stay unpublished: it carries
# no auth of its own and is meant to be reachable only from inside the deployment.
EXPOSE 3339

ENTRYPOINT ["/core-service/bcr-wdc-core-service"]
