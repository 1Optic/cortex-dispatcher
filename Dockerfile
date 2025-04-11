FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.86.0@sha256:5c3eb6f264fe52a95157feda45d4ed7a11170f7d06962339e39cd5526c0a02ce AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian:trixie-slim@sha256:ed637a19d39903303875295de8aacb3131ba17d0d3116f700f05b220da0035d0

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
