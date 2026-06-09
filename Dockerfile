# ==========================================
# STAGE 0: COMPILE RUST (Quay.io + musl target)
# ==========================================
FROM rust:1.79-bookworm

# Install build dependencies untuk static linking
RUN apt-get update && apt-get install -y \
    gcc \
    musl-tools \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Tambah target musl biar binary jadi static
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /app
COPY . .
RUN rm -f /app/src-server/Cargo.lock

WORKDIR /app/src-server
RUN cargo build --release --target x86_64-unknown-linux-musl --target-dir /app/target

# ==========================================
# STAGE 1: RUNTIME (Alpine + PHP)
# ==========================================
FROM alpine:latest

RUN apk add --no-cache \
    php83 php83-pdo php83-pdo_mysql php83-json php83-opcache \
    mysql-client ca-certificates openssl tzdata \
    && ln -s /usr/bin/php83 /usr/bin/php

WORKDIR /app
COPY config/ /app/config/
COPY src-worker/ /app/src-worker/
COPY public/ /app/public/

RUN printf '%s\n' \
  'server:' \
  '  host: "0.0.0.0"' \
  '  port: 8080' \
  '  https_port: 8443' \
  '  tls:' \
  '    enabled: false' \
  '    cert_path: "/app/certs/cert.pem"' \
  '    key_path: "/app/certs/key.pem"' \
  '' \
  'php:' \
  '  docroot: "/app/public"' \
  '  worker_path: "/app/src-worker/worker.php"' \
  '  worker_count: 32' \
  '  memory_limit_mb: 128' \
  '  max_requests: 5000' \
  '  timeout_ms: 30000' \
  '  connection_pool_size: 5' \
  '' \
  'socket:' \
  '  directory: "/tmp/bakpiarun"' \
  '' \
  'logging:' \
  '  access_log_enabled: true' \
  '  access_log: "/dev/stdout"' \
  '  error_log_enabled: true' \
  '  error_log: "/dev/stderr"' \
  '  level: "info"' \
  '  file: "/dev/stdout"' \
  '' \
  'rate_limit:' \
  '  enabled: false' \
  '  requests_per_minute: 120' \
  '  burst_size: 20' \
  '' \
  'security:' \
  '  x_frame_options: "DENY"' \
  '  x_content_type_options: true' \
  '  x_xss_protection: true' \
  '  referrer_policy: "strict-origin-when-cross-origin"' \
  '' \
  'compression:' \
  '  enabled: true' \
  '  min_size_bytes: 1024' \
  '  level: 6' \
  > /app/config/bakpiarun.yaml

COPY --from=0 /app/target/x86_64-unknown-linux-musl/release/bakpiarun-server /app/bakpiarun-server
RUN chgrp -R 0 /app && chmod -R g=u /app
EXPOSE 8080
CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]