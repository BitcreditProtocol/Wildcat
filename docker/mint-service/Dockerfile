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

WORKDIR /mint-service
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-mint-service .

# Expose web server port
EXPOSE 3338
# The admin listener (admin_bind_address, set via
# MINT_SERVICE__ADMIN_BIND_ADDRESS, no default) carries no auth of its own
# and must stay reachable only from inside the deployment: deliberately
# not EXPOSEd, so `docker run -P` cannot publish it. See
# docs/admin-listener.md.

ENTRYPOINT ["/mint-service/bcr-wdc-mint-service"]
