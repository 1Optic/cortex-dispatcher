FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.81.0@sha256:04ffe5b834a36269000d7baf78f11acb174664dfac95470465ef385e2a4330ac AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian:trixie-slim

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
