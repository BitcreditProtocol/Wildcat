FROM wildcat/base-image AS rust-builder

##############################
## Create image
##############################
FROM debian:bookworm-20250224-slim@sha256:12c396bd585df7ec21d5679bb6a83d4878bc4415ce926c9e5ea6426d23c60bdc
RUN apt-get update && apt install -y openssl

WORKDIR /quote-service

COPY --from=rust-builder /wildcat/target/release/bcr-wdc-quote-service .

# Expose web server port
EXPOSE 3338

CMD ["/quote-service/bcr-wdc-quote-service"]
