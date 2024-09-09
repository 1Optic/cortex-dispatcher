FROM debian:stable-slim@sha256:64bc71feaa7ec2ac758a6a3a37c0f0d6ebccf0a45e3f5af1f1d3b5d4cb316b29

RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*

COPY target/release/cortex-dispatcher /usr/bin/
ENTRYPOINT ["/usr/bin/cortex-dispatcher"]
