FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.82.0@sha256:7e4db198c994923452243ec75ff6fb7a943459a88d59e07ef68089520126bae2 AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian:trixie-slim@sha256:ed637a19d39903303875295de8aacb3131ba17d0d3116f700f05b220da0035d0

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
