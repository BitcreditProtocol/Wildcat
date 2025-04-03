FROM wildcat/base-image AS rust-builder

##############################
## Create image
##############################
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install --quiet --no-install-recommends --no-install-suggests --yes openssl curl ca-certificates \
  && apt-get clean \
  && rm --recursive --force /var/lib/apt/lists/*

WORKDIR /ebpp
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-ebpp .

# Expose gRPC port
EXPOSE 9090

CMD ["/ebpp/bcr-wdc-ebpp"]
