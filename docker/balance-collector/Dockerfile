ARG WILDCAT_BASE_IMAGE=wildcat/base-image
FROM ${WILDCAT_BASE_IMAGE} AS rust-builder

##############################
## Create image
##############################
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install --quiet --no-install-recommends --no-install-suggests --yes openssl curl ca-certificates \
  && apt-get clean \
  && rm --recursive --force /var/lib/apt/lists/*

WORKDIR /balance-collector
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-balance-collector .

# Expose web server port
EXPOSE 3338

CMD ["/balance-collector/bcr-wdc-balance-collector"]
