# syntax=docker/dockerfile:1
#
# BrowserOS neo on a virtual display, streamed to a browser tab over noVNC,
# with Neo's MCP server reachable from outside the container.
#
#   docker build --no-cache-filter neo --build-arg NEO_VERSION=0.51.2.5 -t neo .
#
# The runtime contract (ports 6080/9010, /data, env vars) is documented in
# README.md; entrypoint.sh implements it. Use Neo 0.51.2.5 or later: older
# browsers ignore the switch that gives the viewer its minimal frame.

# Optional local package. Empty by default; `--build-context neo-deb=<dir>` with
# one .deb in <dir> installs that build instead of downloading NEO_VERSION. This
# is how CI artifacts and drafts that never reached the CDN get tested.
FROM scratch AS neo-deb

# An optional build context containing <arch>/browseros-claw-server-rs.
# Releases supply this commit's server; standalone builds keep the .deb's server.
FROM scratch AS neo-server

FROM ubuntu:24.04 AS neo-runtime

ARG TARGETARCH
ARG NEO_VERSION=""

LABEL org.opencontainers.image.source="https://github.com/browseros-ai/BrowserOS" \
      org.opencontainers.image.title="BrowserOS neo" \
      org.opencontainers.image.description="BrowserOS neo streamed over noVNC, with its MCP server exposed" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"

SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ENV DEBIAN_FRONTEND=noninteractive

# libasound2t64 must be installed before Neo: the .deb's unversioned
# `Depends: libasound2` otherwise resolves to an OSS shim and Neo exits 127.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl jq tini \
      tigervnc-standalone-server tigervnc-tools openbox novnc websockify \
      libasound2t64 fonts-liberation fonts-noto-core fonts-noto-color-emoji fonts-wqy-zenhei fonts-ipafont-gothic \
 && rm -rf /var/lib/apt/lists/*

# The CDN publishes one release.json per Linux release listing each package's
# URL and sha256, keyed x64_deb / arm64_deb. Docker's TARGETARCH says amd64
# where the release says x64. The local package is bind-mounted rather than
# copied so it never becomes an image layer.
RUN --mount=type=bind,from=neo-deb,target=/tmp/neo-local \
    set -eu; \
    if compgen -G "/tmp/neo-local/*.deb" >/dev/null; then \
      deb=$(ls /tmp/neo-local/*.deb); \
      [ "$(wc -w <<<"$deb")" -eq 1 ] || { echo "neo-deb context must hold exactly one .deb" >&2; exit 1; }; \
      arch=$(dpkg-deb -f "$deb" Architecture); \
      [ "$arch" = "$TARGETARCH" ] || { echo "$(basename "$deb") is $arch, building $TARGETARCH" >&2; exit 1; }; \
      cp "$deb" /tmp/neo.deb; \
    else \
      [ -n "$NEO_VERSION" ] || { echo "Set --build-arg NEO_VERSION=<version>" >&2; exit 1; }; \
      key=$([ "$TARGETARCH" = amd64 ] && echo x64_deb || echo "${TARGETARCH}_deb"); \
      meta=$(curl -fsSL "https://cdn.browseros.com/releases/browserclaw/${NEO_VERSION}/linux/release.json") \
        || { echo "Neo ${NEO_VERSION} has no Linux release on the CDN" >&2; exit 1; }; \
      url=$(jq -r --arg k "$key" '.artifacts[$k].url // empty' <<<"$meta" | sed 's#^http://#https://#'); \
      sha=$(jq -r --arg k "$key" '.artifacts[$k].sha256 // empty' <<<"$meta"); \
      [ -n "$url" ] && [ -n "$sha" ] || { echo "Neo ${NEO_VERSION} has no ${key} package" >&2; exit 1; }; \
      curl -fsSL -o /tmp/neo.deb "$url"; \
      echo "${sha}  /tmp/neo.deb" | sha256sum -c -; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends /tmp/neo.deb; \
    rm -rf /tmp/neo.deb /var/lib/apt/lists/*

# The browser launches the bundled path, so replace that executable in place.
# A changed package layout must fail instead of silently shipping an old server.
RUN --mount=type=bind,from=neo-server,target=/tmp/neo-server \
    set -eu; \
    if [ -n "$(ls -A /tmp/neo-server)" ]; then \
      paths=$(find /usr/lib /opt -type f -name browseros-claw-server); \
      test -n "$paths" && test "$(printf '%s\n' "$paths" | wc -l)" -eq 1; \
      install -m 0755 "/tmp/neo-server/$TARGETARCH/browseros-claw-server-rs" "$paths"; \
    fi

# Ubuntu's image ships an `ubuntu` user at uid 1000; Neo's user takes that uid
# so bind-mounted folders match the usual first user on Linux hosts.
RUN userdel -r ubuntu \
 && useradd -m -u 1000 -s /bin/bash neo

COPY openbox-rc.xml /etc/neo/openbox-rc.xml
# Server behaviour knobs live in this file rather than in more environment
# variables. The shipped copy only documents the defaults; deployments mount or
# COPY their own over it (README).
COPY neo.toml /etc/neo/neo.toml
COPY novnc-index.html /usr/share/novnc/index.html
COPY --chmod=0755 entrypoint.sh /usr/local/bin/neo-entrypoint
COPY --chmod=0755 healthcheck.sh /usr/local/bin/neo-healthcheck
COPY --chmod=0755 neo-quit.py /usr/local/bin/neo-quit

# CLAW_ANALYTICS_ENABLED=0 turns off the bundled server's product analytics:
# every container would otherwise report to the desktop PostHog project as a
# separate install. The browser's native metrics do not read it.
ENV NEO_SCREEN=1600x1000 \
    NEO_DATA_DIR=/data \
    BROWSERCLAW_HOSTED_MODE=1 \
    BROWSERCLAW_CONFIG_FILE=/etc/neo/neo.toml \
    CLAW_ANALYTICS_ENABLED=0 \
    NEO_SANDBOX=false \
    NEO_SOFTWARE_GRAPHICS=true \
    BROWSEROS_CAPSOLVER_API_KEY_FILE=/home/neo/.config/browseros/captcha/capsolver-api-key \
    BROWSEROS_2CAPTCHA_API_KEY_FILE=/home/neo/.config/browseros/captcha/2captcha-api-key \
    NEO_DISABLE_BROWSEROS_EXTENSIONS=true \
    NEO_ARGS="" \
    PUID=1000 \
    PGID=1000

# Callers choose their persistent mount; declaring VOLUME here would also create
# an anonymous /data volume for every image built FROM Neo.
EXPOSE 6080 9010

HEALTHCHECK --interval=5s --timeout=5s --start-period=90s --retries=3 \
  CMD ["/usr/local/bin/neo-healthcheck"]

# tini reaps orphans and forwards `docker stop`'s SIGTERM to the entrypoint,
# which gives Neo time to flush cookies before the container exits.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/neo-entrypoint"]

# The release CLI always rebuilds this final stage (--no-cache-filter neo), even
# without keys. Caching it could retain rotated/removed secret contents.
FROM neo-runtime AS neo

# Keys are intentionally part of the final image and readable by image users.
# BuildKit keeps their input values out of arguments/logs; it does not hide the
# installed files. Rebuild without layer reuse when rotating either secret.
RUN --mount=type=secret,id=capsolver_api_key,required=false \
    --mount=type=secret,id=twocaptcha_api_key,required=false \
    set -eu; \
    destination=/home/neo/.config/browseros/captcha; \
    install -d -m 0700 -o neo -g neo "$destination"; \
    if [ -s /run/secrets/capsolver_api_key ]; then \
      install -m 0400 -o neo -g neo /run/secrets/capsolver_api_key "$destination/capsolver-api-key"; \
    fi; \
    if [ -s /run/secrets/twocaptcha_api_key ]; then \
      install -m 0400 -o neo -g neo /run/secrets/twocaptcha_api_key "$destination/2captcha-api-key"; \
    fi
