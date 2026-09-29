#!/bin/sh
# Starts the link service (server/relay.ts, bundled) beside nginx, which
# forwards /sync to it on 127.0.0.1:8002, and starts it again if it stops.
(
  while :; do
    node /opt/birdsynth/relay.cjs
    echo "birdsynth link service stopped ($?); starting it again" >&2
    sleep 1
  done
) &
# nginx starts when this returns: give the service up to 3 s to listen first
i=0
while [ "$i" -lt 30 ] && ! wget -q -O /dev/null http://127.0.0.1:8002/sync/health 2>/dev/null; do
  i=$((i + 1))
  sleep 0.1
done
