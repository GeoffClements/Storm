FROM debian:stable-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        cmake \
        gcc \
        libc6-dev \
        make \
        pkg-config \
        g++-arm-linux-gnueabi \
        libc6-dev-armel-cross \
        qemu

RUN dpkg --add-architecture armel && \
    apt-get update && \
    apt install -y --no-install-recommends \
        libgstreamer1.0-dev:armel \
        libgstreamer-plugins-base1.0-dev:armel \
        gstreamer1.0-plugins-base:armel \
        gstreamer1.0-plugins-good:armel \
        gstreamer1.0-libav libgstrtspserver-1.0-dev:armel \
        pkg-config:armel

ENV CARGO_TARGET_ARM_UNKNOWN_LINUX_GNUEABI_LINKER=arm-linux-gnueabi-gcc \
    CARGO_TARGET_ARM_UNKNOWN_LINUX_GNUEABI_RUNNER=qemu-arm \
    CC_arm_unknown_linux_gnueabi=arm-linux-gnueabi-gcc \
    CXX_arm_unknown_linux_gnueabi=arm-linux-gnueabi-g++ \
    OPENSSL_DIR=/openssl \
    OPENSSL_INCLUDE_DIR=/openssl/include \
    OPENSSL_LIB_DIR=/openssl/lib \
    QEMU_LD_PREFIX=/usr/arm-linux-gnueabi \
    RUST_TEST_THREADS=1
