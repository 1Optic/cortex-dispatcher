FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.100.1@sha256:cf2d6767de5db8063565f2ac496f256c87fe7e7d3aeaa8a6679746a0f183cb44 AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian@sha256:9c6e381421d0f48d1b8133e573f33c185ee16cbf505eda9c4a8a4791b25ea8a6

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
