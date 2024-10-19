FROM ubuntu:24.04@sha256:99c35190e22d294cdace2783ac55effc69d32896daaa265f0bbedbcde4fbe3e5

RUN apt-get update && apt-get install -y \
    python3 \
    python3-pip

RUN python3 -m pip install --upgrade pip

RUN pip3 install pysftp

COPY data-generator /usr/bin/

ENTRYPOINT ["/usr/bin/data-generator"]
