
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

build-docker-images: build-docker-key-service build-docker-treasury-service build-docker-swap-service build-docker-quote-service
