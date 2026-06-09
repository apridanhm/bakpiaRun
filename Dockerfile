# ==========================================
# STAGE 1: COMPILE RUST (Alpine Build)
# ==========================================
FROM rust:1.75-alpine3.19 AS builder

# Install build dependencies
RUN apk add --no-cache gcc musl-dev pkgconfig openssl-dev

WORKDIR /app
COPY . .

# FIX: Build spesifik package "bakpiarun-server" dari workspace
RUN cargo build --release --package bakpiarun-server --target-dir /app/target

# FIX: Copy binary dengan nama yang sesuai
COPY --from=builder /app/target/release/bakpiarun-server /app/bakpiarun-server

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

# Copy application files
COPY config/ /app/config/
COPY src-worker/ /app/src-worker/
COPY public/ /app/public/

# Copy compiled binary from stage 1
COPY --from=builder /app/bakpiarun-server /app/bakpiarun-server

# FIX OPENSHIFT SCC: Allow arbitrary user ID (WAJIB!)
RUN chgrp -R 0 /app && chmod -R g=u /app

EXPOSE 8080

# Run the server
CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]