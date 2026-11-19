# OK3568 aarch64 交叉编译镜像（docs/ops/ok3568-build.md §5，cross.toml 引用）
#
# 用法：
#   docker build -f scripts/cross-ok3568/Dockerfile -t landesk-cross .
#   docker run --rm -it \
#       -v $(pwd):/src -v /home/.cargo:/root/.cargo \
#       -w /src landesk-cross \
#       cargo build --release --target aarch64-unknown-linux-gnu \
#         -p controlled -p controller \
#         --features "capture/linux,avcodec-wrap/v4l2m2m,avcodec-wrap/openh264"
#
# 说明：
# - 镜像内预装 aarch64 交叉工具链（Debian aarch64 cross gcc + binutils）；
# - 裁剪 libav / OpenH264 的产物（thirdparty/av-aarch64）随源码卷挂载进容器，
#   由 build.rs 经 LAVF_ROOT/OPENH264_ROOT 环境变量探测（见 docs/ops/ok3568-build.md §3）。
FROM rust:1.94-bookworm

# aarch64 交叉编译工具链（Debian 包名）
RUN apt-get update && apt-get install -y --no-install-recommends \
        gcc-aarch64-linux-gnu \
        binutils-aarch64-linux-gnu \
        pkg-config-aarch64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*

# rustup target（.cargo/config.toml 的 linker 已指向 aarch64-linux-gnu-gcc）
RUN rustup target add aarch64-unknown-linux-gnu

WORKDIR /src
