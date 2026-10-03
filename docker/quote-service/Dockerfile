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

WORKDIR /quote-service
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-quote-service .

# Expose web server port
EXPOSE 3338
# Admin port (admin_bind_address, set via QUOTE_SERVICE__ADMIN_BIND_ADDRESS, has no
# default and must be set explicitly) must stay unpublished: it carries no auth of
# its own and is meant to be reachable only from inside the deployment. See
# docs/admin-listener.md.
EXPOSE 3339

ENTRYPOINT ["/quote-service/bcr-wdc-quote-service"]
