FROM ubuntu:24.04@sha256:1e622c5f073b4f6bfad6632f2616c7f59ef256e96fe78bf6a595d1dc4376ac02

RUN apt-get update && apt-get install -y \
    python3 \
    python3-pip

RUN python3 -m pip install --upgrade pip

RUN pip3 install pysftp

COPY data-generator /usr/bin/

ENTRYPOINT ["/usr/bin/data-generator"]
