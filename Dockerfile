FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.94.2@sha256:720fd20d0093d964ab7bbd0d16bfdf4cd486278f07cbe4bbde813c8946fbc0a9 AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian:trixie-slim@sha256:26f98ccd92fd0a44d6928ce8ff8f4921b4d2f535bfa07555ee5d18f61429cf0c

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
