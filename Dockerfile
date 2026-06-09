# ==========================================
# STAGE 1: COMPILE RUST (Debian-based, Latest)
# ==========================================
FROM rust:latest AS builder

# Install build dependencies (Debian)
RUN apt-get update && apt-get install -y \
    gcc \
    musl-tools \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy semua file
WORKDIR /app
COPY . .

# Hapus Cargo.lock yang mungkin incompatible
RUN rm -f /app/src-server/Cargo.lock

# Build dari src-server
WORKDIR /app/src-server
RUN cargo build --release --target-dir /app/target

# ==========================================
# STAGE 2: RUNTIME (Alpine - Kecil & Cepat)
# ==========================================
FROM alpine:latest

# Install PHP + MySQL client + runtime deps
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

# Copy application files
COPY config/ /app/config/
COPY src-worker/ /app/src-worker/
COPY public/ /app/public/

# Copy compiled binary dari stage 1 (Debian) ke stage 2 (Alpine)
COPY --from=builder /app/target/release/bakpiarun-server /app/bakpiarun-server

# FIX OPENSHIFT SCC: Allow arbitrary user ID (WAJIB!)
RUN chgrp -R 0 /app && chmod -R g=u /app

EXPOSE 8080

# Run the server
CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]