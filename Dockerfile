# =====================================================================
# STAGE 0: BUILDER (Alpine + Rust Stable musl)
# Mendukung multi-arch (x86_64 & arm64/Apple Silicon) dengan ABI identik
# =====================================================================
FROM alpine:3.21 AS builder

RUN apk add --no-cache \
    curl \
    build-base \
    musl-dev \
    pkgconf \
    openssl-dev \
    openssl-libs-static \
    cmake \
    clang \
    perl \
    linux-headers \
    ca-certificates

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    RUSTFLAGS="-C target-feature=-crt-static" \
    OPENSSL_NO_VENDOR=1

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal

WORKDIR /build/src-server

# 1. Cache dependency crates terlebih dahulu agar rebuild cepat
COPY src-server/Cargo.toml src-server/Cargo.lock ./
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src target/release/deps/bakpiarun_server* target/release/bakpiarun-server*

# 2. Compile source code utama BakpiaRun Server
COPY src-server/src ./src
RUN cargo build --release \
    && strip target/release/bakpiarun-server

# =====================================================================
# STAGE 1: RUNTIME (Alpine 3.21 + Persistent PHP 8.3 CLI + OPcache)
# =====================================================================
FROM alpine:3.21 AS runtime

RUN apk add --no-cache \
    libgcc \
    libssl3 \
    libcrypto3 \
    ca-certificates \
    openssl \
    curl \
    tzdata \
    mysql-client \
    php83 \
    php83-pcntl \
    php83-posix \
    php83-sockets \
    php83-pdo \
    php83-pdo_mysql \
    php83-mysqli \
    php83-mysqlnd \
    php83-curl \
    php83-mbstring \
    php83-openssl \
    php83-session \
    php83-ctype \
    php83-dom \
    php83-xml \
    php83-fileinfo \
    php83-opcache \
    && ([ -e /usr/bin/php ] || ln -s /usr/bin/php83 /usr/bin/php)

# Aktifkan Zend OPcache pada mode PHP CLI untuk performa maksimal worker persisten
RUN printf '%s\n' \
    'opcache.enable=1' \
    'opcache.enable_cli=1' \
    'opcache.memory_consumption=128' \
    'opcache.interned_strings_buffer=16' \
    'opcache.max_accelerated_files=10000' \
    'opcache.validate_timestamps=0' \
    > /etc/php83/conf.d/99-bakpiarun-opcache.ini

WORKDIR /app

COPY --from=builder /build/src-server/target/release/bakpiarun-server /app/bakpiarun-server
COPY config/bakpiarun.docker.yaml /app/config/bakpiarun.yaml
COPY src-worker/ /app/src-worker/
COPY public/ /app/public/
COPY lib/ /app/lib/
COPY examples/ /app/examples/

# Siapkan direktori socket, upload, log, sertifikat fallback, & permission grup 0 (Kubernetes/OpenShift ready)
RUN mkdir -p /app/certs /app/logs /tmp/bakpiarun /tmp/bakpiarun_uploads \
    && openssl req -x509 -newkey rsa:2048 -nodes \
       -keyout /app/certs/key.pem \
       -out /app/certs/cert.pem \
       -days 365 \
       -subj "/CN=localhost" >/dev/null 2>&1 \
    && chgrp -R 0 /app /tmp/bakpiarun /tmp/bakpiarun_uploads \
    && chmod -R g=u /app /tmp/bakpiarun /tmp/bakpiarun_uploads \
    && chmod +x /app/bakpiarun-server

EXPOSE 8080 8443

HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS -H "X-Admin-Token: ${BAKPIA_ADMIN_TOKEN:-dev-token-for-local-testing}" http://127.0.0.1:${BAKPIARUN_PORT:-8080}/health >/dev/null || exit 1

CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]
