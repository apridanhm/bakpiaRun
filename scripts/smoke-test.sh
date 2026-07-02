#!/usr/bin/env bash
#
# Bare-image smoke test (§6.1).
#
# Builds the Docker image and runs it WITHOUT any mounted config, proving the
# config baked into the image is valid and the server actually starts. This
# would have caught the previous bug where the baked config omitted the
# required `pools:` / `queue:` sections and the binary crashed on startup.
#
# Usage: scripts/smoke-test.sh
set -euo pipefail

IMAGE="bakpiarun:smoke-test"
NAME="bakpiarun-smoke-$$"
PORT=18080

echo "[smoke] Building image..."
docker build -t "$IMAGE" .

echo "[smoke] Running BARE container (no volume mounts)..."
# BAKPIA_ADMIN_INSECURE=1 lets us reach /health without configuring a token.
docker run -d --rm --name "$NAME" -p "${PORT}:8080" -e BAKPIA_ADMIN_INSECURE=1 "$IMAGE"

cleanup() { docker stop "$NAME" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "[smoke] Waiting for server to become healthy..."
for _ in $(seq 1 30); do
    if curl -fsS "http://localhost:${PORT}/health" >/dev/null 2>&1; then
        echo "[smoke] PASS: bare image started and /health responded."
        curl -s "http://localhost:${PORT}/health" | head -c 500; echo
        exit 0
    fi
    sleep 1
done

echo "[smoke] FAIL: server did not become healthy. Recent container logs:"
docker logs "$NAME" 2>&1 | tail -50
exit 1
