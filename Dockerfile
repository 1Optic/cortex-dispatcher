FROM debian:stable-slim@sha256:fffe16098bcefa876d01862a61f8f30ef4292c9485940e905d41a15d8459828b

LABEL org.opencontainers.image.source="https://gitlab.1optic.io/hitc/cortex-dispatcher"

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY target/release/cortex-dispatcher /usr/bin/
ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
