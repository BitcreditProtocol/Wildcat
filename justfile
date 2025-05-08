
# print available targets
[group("project-agnostic")]
default:
    @just --list --justfile {{justfile()}}

# evaluate and print all just variables
[group("project-agnostic")]
evaluate:
    @just --evaluate

# print system information such as OS and architecture
[group("project-agnostic")]
system-info:
  @echo "architecture: {{arch()}}"
  @echo "os: {{os()}}"
  @echo "os family: {{os_family()}}"

build-docker-base-image:
    docker build -t wildcat/base-image -f docker/base-image/Dockerfile .

build-docker-key-service: build-docker-base-image
    docker build -t wildcat/key-service -f docker/key-service/Dockerfile .

build-docker-treasury-service: build-docker-base-image
    docker build -t wildcat/treasury-service -f docker/treasury-service/Dockerfile .

build-docker-swap-service: build-docker-base-image
    docker build -t wildcat/swap-service -f docker/swap-service/Dockerfile .

build-docker-quote-service: build-docker-base-image
    docker build -t wildcat/quote-service -f docker/quote-service/Dockerfile .

build-docker-wallet-aggregator: build-docker-base-image
    docker build -t wildcat/wallet-aggregator -f docker/wallet-aggregator/Dockerfile .

build-docker-ebpp: build-docker-base-image
    docker build -t wildcat/ebpp -f docker/ebpp/Dockerfile .

build-docker-ebill-service: build-docker-base-image
    docker build -t wildcat/ebill-service -f docker/ebill-service/Dockerfile .

build-docker-e2e-tests: build-docker-base-image
    docker build -t wildcat/e2e-tests -f docker/e2e-tests/Dockerfile .

build-docker-images: build-docker-key-service build-docker-treasury-service build-docker-swap-service build-docker-quote-service build-docker-wallet-aggregator build-docker-ebpp build-docker-ebill-service

openapi-generate-docs:
  @cargo run --package bcr-wdc-quote-service --bin gen_api

openapi-validate-docs:
  # make sure you have "schemathesis" installed (`pip3 install schemathesis`)
  @st run openapi.json --dry-run # dry-run: Simulate test execution without making any actual requests, useful for validating data generation
