FROM ubuntu:26.04@sha256:065aa8626841f16057b78b027b464cd59c4fac36c204c72dcbf48d66f5fa8a96

RUN apt-get update && apt-get install -y \
    python3 \
    python3-pip

RUN python3 -m pip install --upgrade pip

RUN pip3 install pysftp

COPY data-generator /usr/bin/

ENTRYPOINT ["/usr/bin/data-generator"]
