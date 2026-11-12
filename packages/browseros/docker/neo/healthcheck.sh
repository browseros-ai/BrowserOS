#!/bin/sh
# Healthy only when an agent and a viewer can both use the container:
# - the entrypoint has applied agent settings (human help off),
# - claw-server is connected to Neo, checked through the MCP proxy on 9010 so
#   the port agents use is part of the check (/system/health would pass while
#   that connection is down; /system/ready returns 503 until it is up),
# - the noVNC page is served.
[ -f /tmp/neo-configured ] \
  && curl -fsS http://127.0.0.1:9010/system/ready >/dev/null \
  && curl -fsS http://127.0.0.1:6080/ >/dev/null
