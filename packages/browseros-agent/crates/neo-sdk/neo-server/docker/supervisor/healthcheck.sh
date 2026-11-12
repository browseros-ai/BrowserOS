#!/bin/sh
# Healthy only when an agent, and a viewer where the container runs one, can use it:
# - the entrypoint has finished configuring (for claw-server, human help off),
# - the server is connected to Neo, checked through the MCP proxy on 9010 so
#   the port agents use is part of the check (/system/health would pass while
#   that connection is down; /system/ready returns 503 until it is up),
# - the noVNC page is served, if the entrypoint started websockify (it then
#   writes /tmp/neo-viewer). claw-sdk images run no viewer.
[ -f /tmp/neo-configured ] \
  && curl -fsS http://127.0.0.1:9010/system/ready >/dev/null \
  && { [ ! -f /tmp/neo-viewer ] || curl -fsS http://127.0.0.1:6080/ >/dev/null; }
