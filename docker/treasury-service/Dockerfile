ARG WILDCAT_BASE_IMAGE=wildcat/base-image
FROM ${WILDCAT_BASE_IMAGE} AS rust-builder

##############################
## Create image
##############################
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install --quiet --no-install-recommends --no-install-suggests --yes openssl curl \
  && apt-get clean \
  && rm --recursive --force /var/lib/apt/lists/*

WORKDIR /treasury-service
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-treasury-service .

# Expose web server port
EXPOSE 3338

CMD ["/treasury-service/bcr-wdc-treasury-service"]
