FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.93.0@sha256:2bbff0694baa63d2f6139db7ae3ce6a5dec685e9b7269974943897d17aaac623 AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian:trixie-slim@sha256:bfc1a095aef012070754f61523632d1603d7508b4d0329cd5eb36e9829501290

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
