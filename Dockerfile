FROM debian:stable-slim

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY target/release/cortex-dispatcher /usr/bin/
ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
