# ==========================================
# STAGE 1: COMPILE RUST (Latest Rust)
# ==========================================
FROM rust:latest-alpine AS builder

# Install build dependencies
RUN apk add --no-cache gcc musl-dev pkgconfig openssl-dev

# Copy semua file
WORKDIR /app
COPY . .

# Hapus Cargo.lock yang mungkin incompatible
RUN rm -f /app/src-server/Cargo.lock

# Build dari src-server
WORKDIR /app/src-server
RUN cargo build --release --target-dir /app/target

# ==========================================
# STAGE 2: RUNTIME (Alpine + PHP)
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

# Copy compiled binary
COPY --from=builder /app/target/release/bakpiarun-server /app/bakpiarun-server

# FIX OPENSHIFT SCC
RUN chgrp -R 0 /app && chmod -R g=u /app

EXPOSE 8080

CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]