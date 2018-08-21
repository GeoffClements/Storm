FROM arm32v7/ubuntu:18.04

RUN apt update && apt -y upgrade \
    && apt install -y --no-install-recommends gcc \
        curl \
        ca-certificates \
        libgstreamer1.0-dev \
        libgstreamer-plugins-base1.0-dev \
        gstreamer1.0-plugins-base \
        gstreamer1.0-plugins-good \
        gstreamer1.0-libav libgstrtspserver-1.0-dev 

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y

WORKDIR /storm
ENTRYPOINT cargo build --release
