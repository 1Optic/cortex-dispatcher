FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.90.0@sha256:15d848efef5b33eb187d4615d570b60fed8b5cc472780a26a22c958fa8c33908 AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian:trixie-slim@sha256:88ef4df0f82963ff3c0472493da188f082822b2a16b1be23d238d124d5c8c92e

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
