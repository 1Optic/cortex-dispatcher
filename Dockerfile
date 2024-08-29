FROM debian:stable-slim@sha256:382967fd7c35a0899ca3146b0b73d0791478fba2f71020c7aa8c27e3a4f26672

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY target/release/cortex-dispatcher /usr/bin/
ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
