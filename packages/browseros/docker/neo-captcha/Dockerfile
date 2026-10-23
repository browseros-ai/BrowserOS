# syntax=docker/dockerfile:1
ARG NEO_IMAGE=ghcr.io/browseros-ai/neo:0.51.2.1
FROM ${NEO_IMAGE}
ARG TARGETARCH

# Locate the bundled server again: a changed base layout must fail the build.
RUN --mount=type=bind,source=servers,target=/tmp/servers \
    set -eu; \
    paths=$(find /usr/lib /opt -type f -name browseros-claw-server); \
    test -n "$paths" && test "$(printf '%s\n' "$paths" | wc -l)" -eq 1; \
    install -m 0755 "/tmp/servers/$TARGETARCH/browseros-claw-server-rs" "$paths"

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

ENV BROWSEROS_CAPSOLVER_API_KEY_FILE=/home/neo/.config/browseros/captcha/capsolver-api-key \
    BROWSEROS_2CAPTCHA_API_KEY_FILE=/home/neo/.config/browseros/captcha/2captcha-api-key \
    NEO_ARGS=--disable-browseros-server-updater
