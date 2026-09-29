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
