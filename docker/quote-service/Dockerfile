FROM wildcat/base-image AS rust-builder

##############################
## Create image
##############################
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install --quiet --no-install-recommends --no-install-suggests --yes openssl curl \
  && apt-get clean \
  && rm --recursive --force /var/lib/apt/lists/*



WORKDIR /quote-service
COPY --from=rust-builder /wildcat/target/release/bcr-wdc-quote-service .

# Expose web server port
EXPOSE 3338

CMD ["/quote-service/bcr-wdc-quote-service"]
