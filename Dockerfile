# ==========================================
# STAGE 1: COMPILE RUST (Alpine Build)
# ==========================================
FROM rust:1.75-alpine3.19 AS builder

# Install build dependencies (gcc, musl-dev, dll)
RUN apk add --no-cache gcc musl-dev pkgconfig openssl-dev

WORKDIR /app
COPY . .

# Compile release binary
# Pastikan nama binary sesuai Cargo.toml lu
RUN cargo build --release --target-dir /app/target

# ==========================================
# STAGE 2: RUNTIME (Alpine + PHP)
# ==========================================
FROM alpine:3.19

# Install PHP + MySQL client + SSL libs (dibutuhkan runtime)
RUN apk add --no-cache \
    php82 \
    php82-pdo \
    php82-pdo_mysql \
    php82-json \
    php82-opcache \
    mysql-client \
    ca-certificates \
    openssl \
    tzdata

# Set PHP config defaults (opsional)
ENV PHP_INI_DIR=/etc/php82

WORKDIR /app

# Copy compiled binary dari stage 1
COPY --from=builder /app/target/release/bakpiarun-server /app/bakpiarun-server

# Copy application files
COPY config/ /app/config/
COPY src-worker/ /app/src-worker/
COPY public/ /app/public/

# FIX OPENSHIFT SCC: Allow arbitrary user ID (WAJIB!)
RUN chgrp -R 0 /app && chmod -R g=u /app

EXPOSE 8080

# Run the server
CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]