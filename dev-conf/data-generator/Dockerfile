FROM ubuntu:24.04@sha256:278628f08d4979fb9af9ead44277dbc9c92c2465922310916ad0c46ec9999295

RUN apt-get update && apt-get install -y \
    python3 \
    python3-pip

RUN python3 -m pip install --upgrade pip

RUN pip3 install pysftp

COPY data-generator /usr/bin/

ENTRYPOINT ["/usr/bin/data-generator"]
