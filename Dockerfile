FROM harbor.hendrikx-itc.nl/1optic/rust-ci:1.99.1@sha256:a318e5705382dddd51d1c448bc72b266f86b2ae3b3a563920c2575aef1056a11 AS build

COPY . /src
WORKDIR /src

RUN cargo build --package cortex-dispatcher --release

FROM debian@sha256:4c25275446ca371766b8b093c696782eabce06263fccacb8d5d813b3dc44b308

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/cortex-dispatcher /usr/bin/

ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
