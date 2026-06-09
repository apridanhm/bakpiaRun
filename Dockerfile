# ==========================================
# STAGE 1: COMPILE RUST (Build Environment)
# ==========================================
# PAKAI IMAGE DARI QUAY.IO (PUBLIC & FREE)
FROM quay.io/rust-lang/rust:1.75.0 AS builder

WORKDIR /app
COPY . .

# Compile release binary
# Pastikan nama package sesuai Cargo.toml lu
RUN cargo build --release --target-dir /app/target

# ==========================================
# STAGE 2: RUNTIME (Production Image)
# ==========================================
FROM registry.access.redhat.com/ubi9/ubi-minimal:latest

# Install PHP + MySQL client
RUN microdnf install -y php php-pdo php-mysqlnd mysql && microdnf clean all

WORKDIR /app

# Copy compiled binary dari stage 1
COPY --from=builder /app/target/release/bakpiarun-server /app/bakpiarun-server

# Copy application files
COPY config/ /app/config/
COPY src-worker/ /app/src-worker/
COPY public/ /app/public/

# FIX OPENSHIFT SCC: Allow arbitrary user ID
RUN chgrp -R 0 /app && chmod -R g=u /app

EXPOSE 8080

# Run the server
CMD ["/app/bakpiarun-server", "--config", "/app/config/bakpiarun.yaml"]