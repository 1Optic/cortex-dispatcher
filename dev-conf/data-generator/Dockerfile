FROM ubuntu:24.04@sha256:4a5ffc07dd015f03d6cb56f209db27ed2378056f7cc8a70ae11d0c35718ed3e6

RUN apt-get update && apt-get install -y \
    python3 \
    python3-pip

RUN python3 -m pip install --upgrade pip

RUN pip3 install pysftp

COPY data-generator /usr/bin/

ENTRYPOINT ["/usr/bin/data-generator"]
