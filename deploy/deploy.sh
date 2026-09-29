#!/usr/bin/env bash
# Build birdsynth, package it for the VM (linux/amd64) and run it there with
# rootless podman compose on port 8001.
#
#   deploy/deploy.sh                 build here and stream the image over SSH
#   HUB=<dockerhub-user> deploy/deploy.sh
#                                    build, push docker.io/<user>/birdsynth, pull on the VM
#
# HOST overrides the target (default birdie@192.168.2.165).
# SKIP_BUILD=1 reuses the existing dist/ instead of running npm run build.
set -euo pipefail
cd "$(dirname "$0")/.."

HOST=${HOST:-birdie@192.168.2.165}
TAG=${TAG:-$(date +%Y%m%d-%H%M%S)}
PORT=8001

if [ -z "${SKIP_BUILD:-}" ]; then
  npm run build
fi
[ -f dist/index.html ] || { echo "dist/ is empty; run npm run build" >&2; exit 1; }
[ -f dist-server/relay.cjs ] || { echo "dist-server/relay.cjs is missing; run npm run build" >&2; exit 1; }

if [ -n "${HUB:-}" ]; then
  REPO="docker.io/$HUB/birdsynth"
  docker buildx build --platform linux/amd64 -f deploy/Containerfile -t "$REPO:$TAG" -t "$REPO:latest" --push .
  PULL=always
else
  REPO="localhost/birdsynth"
  docker buildx build --platform linux/amd64 -f deploy/Containerfile -t "$REPO:$TAG" --load .
  echo "Sending the image to $HOST ..."
  docker save "$REPO:$TAG" | gzip | ssh "$HOST" 'gunzip | podman load -q'
  PULL=never
fi

ssh "$HOST" 'mkdir -p ~/birdsynth'
scp -q deploy/compose.yaml "$HOST:birdsynth/compose.yaml"
ssh "$HOST" bash -s -- "$REPO:$TAG" "$PULL" <<'REMOTE'
set -euo pipefail
cd ~/birdsynth
printf 'BIRDSYNTH_IMAGE=%s\nBIRDSYNTH_PULL=%s\n' "$1" "$2" > .env
out=$(podman compose up -d --remove-orphans 2>&1) || { echo "$out"; exit 1; }
echo "$out" | grep -v 'Executing external compose provider' | grep -v '^[[:space:]]*$' || true
# keep only the image that's running
podman images --format '{{.Repository}}:{{.Tag}}' | grep '/birdsynth:' | grep -vxF "$1" | grep -v ':latest$' | xargs -r podman rmi -f >/dev/null 2>&1 || true
podman ps --filter name=birdsynth --format '{{.Names}}  {{.Image}}  {{.Status}}  {{.Ports}}'
REMOTE

# Check from the VM itself (its firewall may not expose 8001 to the LAN; the
# Cloudflare tunnel reaches it from the VM side).
ssh "$HOST" bash -s -- "$PORT" <<'CHECK'
set -euo pipefail
port=$1
for i in 1 2 3 4 5 6 7 8 9 10; do
  if curl -fsS -o /dev/null "http://127.0.0.1:$port/"; then break; fi
  sleep 1
done
index=$(curl -fsS "http://127.0.0.1:$port/")
wasm=$(printf '%s' "$index" | grep -o 'assets/index-[^"]*\.js' | head -1)
echo "index:  $(curl -fsSI "http://127.0.0.1:$port/" | grep -i '^cache-control' | tr -d '\r')"
js=$(curl -fsS "http://127.0.0.1:$port/$wasm" | grep -o 'assets/engine-[A-Za-z0-9_-]*\.wasm' | head -1)
echo "wasm:   $js"
curl -fsSI -H 'Accept-Encoding: gzip' "http://127.0.0.1:$port/$js" | grep -iE '^(content-type|cache-control|content-encoding)' | tr -d '\r' | sed 's/^/        /'
echo "http→https behind Cloudflare: $(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' -H 'CF-Visitor: {"scheme":"http"}' -H 'Host: birdsynth.example' "http://127.0.0.1:$port/")"
echo "link:   $(curl -fsS "http://127.0.0.1:$port/sync/health" || echo 'not answering')"
CHECK
echo "Deployed $REPO:$TAG to $HOST:$PORT"
