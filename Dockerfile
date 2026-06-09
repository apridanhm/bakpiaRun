# ==========================================
# STAGE 1: COMPILE RUST (Debian + musl target)
# ==========================================
FROM rust:latest AS builder

# Install musl tools buat compile static binary
RUN apt-get update && apt-get install -y \
    gcc \
    musl-tools \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Set target musl & install rustup component
RUN rustup target add x86_64-unknown-linux-musl

# Copy semua file
WORKDIR /app
COPY . .

# Hapus Cargo.lock yang mungkin incompatible
RUN rm -f /app/src-server/Cargo.lock

# BUILD DENGAN MUSL TARGET (static binary!)
WORKDIR /app/src-server
RUN cargo build --release --target x86_64-unknown-linux-musl --target-dir /app/target

# COPY BINARY DARI FOLDER musl-release
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/bakpiarun-server /app/bakpiarun-server

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

# Copy compiled static binary from stage 1
COPY --from=builder /app/bakpiarun-server /app/bakpiarun-server

# FIX OPENSHIFT SCC: Allow arbitrary user ID (WAJIB!)
RUN chgrp -R 0 /app && chmod -R g=u /app

EXPOSE 8080

# Run the server
CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]