FROM ubuntu:24.04@sha256:9d6e6f7d762bf55c4a2f17694dc43d3eefae8452ee70e067d5aa4ddd922fc462

RUN apt-get update && apt-get install -y \
    python3 \
    python3-pip

RUN python3 -m pip install --upgrade pip

RUN pip3 install pysftp

COPY data-generator /usr/bin/

ENTRYPOINT ["/usr/bin/data-generator"]
