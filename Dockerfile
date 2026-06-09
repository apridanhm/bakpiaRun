# ==========================================
# STAGE 1: COMPILE RUST
# ==========================================
FROM rust:1.75-alpine3.19 AS builder

# Install build dependencies
RUN apk add --no-cache gcc musl-dev pkgconfig openssl-dev

# COPY SEMUA SEKALIGUS (biar nggak error pas build context)
WORKDIR /app
COPY . .

# CD KE src-server (tempat Cargo.toml yang valid) & BUILD
WORKDIR /app/src-server
RUN cargo build --release --target-dir /app/target

# ==========================================
# STAGE 2: RUNTIME (Alpine + PHP)
# ==========================================
FROM alpine:3.19

# Install PHP + MySQL client + runtime deps
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

WORKDIR /app

# Copy application files (dari root repo)
COPY config/ /app/config/
COPY src-worker/ /app/src-worker/
COPY public/ /app/public/

# COPY BINARY HASIL COMPILE (dari stage 1)
COPY --from=builder /app/target/release/bakpiarun-server /app/bakpiarun-server

# FIX OPENSHIFT SCC: Allow arbitrary user ID (WAJIB!)
RUN chgrp -R 0 /app && chmod -R g=u /app

EXPOSE 8080

# Run the server
CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]