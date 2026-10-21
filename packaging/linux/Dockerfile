FROM node:24-bookworm-slim AS node
FROM ubuntu:22.04 AS build
COPY --from=node /usr/local /usr/local

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential clang-15 python3 pkg-config libx11-dev ca-certificates curl binutils git \
    libarchive-tools xz-utils \
    && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -fsSf https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain 1.88.0 \
    && rm /tmp/rustup-init.sh
ENV PATH="/root/.cargo/bin:${PATH}"
# Clang 15 支持新版 V8 使用的 std::source_location，运行库仍来自 Ubuntu 22.04。
ENV CC=clang-15 CXX=clang++-15
ENV SUPER_PASTE_GLIBC_MAX=2.35 SUPER_PASTE_GLIBCXX_MAX=3.4.30
ENV ONNXRUNTIME_NODE_INSTALL=skip
RUN corepack enable pnpm && corepack prepare pnpm@11.4.0 --activate

WORKDIR /build
COPY . .
# 使用容器的系统库重编译，避免携带宿主机 GLIBC_2.44 的符号要求。
RUN pnpm install --frozen-lockfile
RUN pnpm exec electron-rebuild --force --build-from-source \
       --only 'better-sqlite3,@paymoapp/active-window' \
    && pnpm run build
RUN pnpm exec electron-builder --linux --publish never

FROM scratch AS artifacts
COPY --from=build /build/dist/*.deb /
COPY --from=build /build/dist/*.pacman /
COPY --from=build /build/dist/*.AppImage /
COPY --from=build /build/dist/linux-unpacked /linux-unpacked
