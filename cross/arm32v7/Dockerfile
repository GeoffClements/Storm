# FROM arm32v7/ubuntu:18.04

# RUN apt update && apt -y upgrade \
#     && apt install -y --no-install-recommends gcc \
#         curl \
#         ca-certificates \
#         libgstreamer1.0-dev \
#         libgstreamer-plugins-base1.0-dev \
#         gstreamer1.0-plugins-base \
#         gstreamer1.0-plugins-good \
#         gstreamer1.0-libav libgstrtspserver-1.0-dev 

# RUN curl https://sh.rustup.rs -sSf | sh -s -- -y

FROM docker.io/ubuntu:18.04

RUN echo "deb [arch=armhf] http://ports.ubuntu.com/ubuntu-ports bionic main" >> /etc/apt/sources.list \
    && echo 'APT::Install-Recommends "0";' >> /etc/apt/apt.conf.d/10local \
    && echo 'APT::Install-Suggests "0";' >> /etc/apt/apt.conf.d/10local \
    && echo "foreign-architecture armhf" >> /etc/dpkg/dpkg.cfg.d/multiarch \
    && apt update

RUN apt-get install -y --no-install-recommends \
    ca-certificates \
    cmake \
    gcc \
    libc6-dev \
    make \
    pkg-config

RUN apt-get install -y --no-install-recommends \
    g++-arm-linux-gnueabi \
    libc6-dev-armel-cross

RUN dpkg --add-architecture arm64 && \
    apt-get update && \
    apt-get install libfoo:arm64

ENV CARGO_TARGET_ARM_UNKNOWN_LINUX_GNUEABI_LINKER=arm-linux-gnueabi-gcc \
    CARGO_TARGET_ARM_UNKNOWN_LINUX_GNUEABI_RUNNER=qemu-arm \
    CC_arm_unknown_linux_gnueabi=arm-linux-gnueabi-gcc \
    CXX_arm_unknown_linux_gnueabi=arm-linux-gnueabi-g++ \
    OPENSSL_DIR=/openssl \
    OPENSSL_INCLUDE_DIR=/openssl/include \
    OPENSSL_LIB_DIR=/openssl/lib \
    QEMU_LD_PREFIX=/usr/arm-linux-gnueabi \
    RUST_TEST_THREADS=1
