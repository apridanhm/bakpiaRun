# ==========================================
# STAGE 1: COMPILE RUST (Debian + musl target)
# ==========================================
FROM rust:latest AS rust-compile-stage

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

# Hapus lock file lama biar nggak conflict versi
RUN rm -f /app/src-server/Cargo.lock

# Compile binary static
WORKDIR /app/src-server
RUN cargo build --release --target x86_64-unknown-linux-musl --target-dir /app/target

# ==========================================
# STAGE 2: RUNTIME (Alpine + PHP)
# ==========================================
FROM alpine:latest

# Install PHP 8.3 + MySQL client
RUN apk add --no-cache \
    php83 \
    php83-pdo \
    php83-pdo_mysql \
    php83-json \
    php83-opcache \
    mysql-client \
    ca-certificates \
    openssl \
    tzdata

WORKDIR /app

# Copy asset aplikasi
COPY config/ /app/config/
COPY src-worker/ /app/src-worker/
COPY public/ /app/public/


RUN mkdir -p /tmp/bakpiarun
# FIX CONFIG: PAKAI printf (BUILDAAH COMPATIBLE!) 
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

# Copy binary static dari Stage 1
COPY --from=rust-compile-stage /app/target/x86_64-unknown-linux-musl/release/bakpiarun-server /app/bakpiarun-server

#  FIX OPENSHIFT SCC: Izinkan arbitrary non-root UID
RUN chgrp -R 0 /app && chmod -R g=u /app

EXPOSE 8080

CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]