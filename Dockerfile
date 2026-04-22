FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.95.0.0@sha256:794278e69ec7190dbf95c6f4fbaec8527d5e030a498abbac2f5bca2ab70d9d55 AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian:trixie-slim@sha256:cedb1ef40439206b673ee8b33a46a03a0c9fa90bf3732f54704f99cb061d2c5a

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
