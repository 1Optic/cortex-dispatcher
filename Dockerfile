FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.98.1@sha256:0b752e4d74e1e80c1a3721d7adefa81fef02d65be371e43d5c344dde844138a6 AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian@sha256:181ecf074fdc824a42be4f84a7be2eba33c4ac298ae1ab15a4e69ed052bd9ec0

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
