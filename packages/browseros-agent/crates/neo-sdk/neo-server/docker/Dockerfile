# syntax=docker/dockerfile:1
#
# The BrowserOS Neo SDK image: the released neo browser plus this commit's SDK server
# (browseros-claw-sdk, one MCP tool: `run`), for hosts that build on it (zuzu's cell image) or
# run it alone. README.md beside this file is the host contract. Build it with build.sh, which
# supplies the two contexts:
#
#   context     packages/browseros-agent    the Rust workspace, filtered by Dockerfile.dockerignore
#   neo-docker  packages/browseros/docker/neo  the browser supervisor shared with the Neo image
#
#   stage server          (build machine's arch)  workspace ─▶ /out/browseros-claw-sdk
#   stage neo-sdk-runtime (target arch)           ubuntu + pinned browser .deb + server
#                                                 + shared supervisor + provenance
#   stage neo-sdk-keyed   neo-sdk-runtime + CAPTCHA keys from BuildKit secrets (build.sh --*-key-file)
#   stage neo-sdk         neo-sdk-runtime as is: keyless, the default target

ARG UBUNTU_IMAGE=ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852
ARG RUST_IMAGE=rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922

# build.sh replaces this empty stage with packages/browseros/docker/neo. Built without it, the
# COPY --from=neo-docker lines below fail with "not found" instead of shipping no supervisor.
FROM scratch AS neo-docker

# The server, cross-compiled on the build machine's own architecture exactly as
# release-docker.yml's server job does (Rust 1.98.0, ziglang 0.15.2, cargo-zigbuild 0.23.0,
# glibc 2.17 floor), so an amd64 image builds on an Apple silicon Mac without emulating rustc.
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS server
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends python3-venv && rm -rf /var/lib/apt/lists/* \
 && python3 -m venv /opt/zig && /opt/zig/bin/pip install --no-cache-dir ziglang==0.15.2 \
 && ln -s "$(/opt/zig/bin/python -c 'import pathlib, ziglang; print(pathlib.Path(ziglang.__file__).parent)')/zig" /usr/local/bin/zig \
 && cargo install --locked cargo-zigbuild --version 0.23.0
WORKDIR /src
COPY . .
# The target cache is per architecture and shared by every checkout built on this Docker daemon,
# so third-party crates compile once; the registry is shared by both architectures.
# build-server.sh cleans the workspace's own crates from it before each build, because Cargo judges
# freshness by mtime and once shipped another checkout's server, and it fails unless the binary
# was compiled from these sources. NEO_SDK_CACHE_SCOPE (build.sh: the env var of that name) gives a
# build a cache of its own, for measurements; NEO_SDK_BUILD_JOBS caps Cargo's parallel jobs.
ARG NEO_SDK_CACHE_SCOPE=
ARG NEO_SDK_BUILD_JOBS=
RUN --mount=type=cache,id=neo-sdk-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=neo-sdk-target-${TARGETARCH}${NEO_SDK_CACHE_SCOPE},target=/src/target,sharing=locked \
    set -eu; \
    case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-gnu ;; \
      arm64) target=aarch64-unknown-linux-gnu ;; \
      *) echo "no server build for $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    rustup target add "$target"; \
    if [ -n "$NEO_SDK_BUILD_JOBS" ]; then export CARGO_BUILD_JOBS="$NEO_SDK_BUILD_JOBS"; fi; \
    bash crates/neo-sdk/neo-server/docker/build-server.sh "$target"

FROM ${UBUNTU_IMAGE} AS neo-sdk-runtime
ARG TARGETARCH
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ENV DEBIAN_FRONTEND=noninteractive

# No novnc/websockify: an SDK image serves no viewer. python3 is listed itself because neo-quit
# (a clean CDP Browser.close at shutdown, so cookies are flushed) needs it, and only websockify
# pulled it into the Neo image.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl jq tini python3 \
      tigervnc-standalone-server tigervnc-tools openbox \
      libasound2t64 fonts-liberation fonts-noto-core fonts-noto-color-emoji fonts-wqy-zenhei fonts-ipafont-gothic \
 && rm -rf /var/lib/apt/lists/*

# The browser is pinned by version and per-architecture sha256 in browser.lock, so a bump is a
# reviewed one-file diff and a changed download fails the build.
COPY crates/neo-sdk/neo-server/docker/browser.lock /usr/share/neo-sdk/browser.lock
RUN set -eu; . /usr/share/neo-sdk/browser.lock; \
    case "$TARGETARCH" in \
      amd64) url=$NEO_BROWSER_URL_AMD64 sha=$NEO_BROWSER_SHA256_AMD64 ;; \
      arm64) url=$NEO_BROWSER_URL_ARM64 sha=$NEO_BROWSER_SHA256_ARM64 ;; \
      *) echo "no browser for $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/neo.deb "$url"; \
    echo "$sha  /tmp/neo.deb" | sha256sum -c -; \
    apt-get update; apt-get install -y --no-install-recommends /tmp/neo.deb; \
    rm -rf /tmp/neo.deb /var/lib/apt/lists/*

# The server goes where the browser launches its bundled one from, and the build fails unless
# the installed binary's version marker says claw-sdk: the shared entrypoint picks the display,
# the host socket and the settings call from that marker at every start.
COPY --from=neo-docker --chmod=0755 neo-server-info.sh /usr/local/bin/neo-server-info
COPY --from=server /out/browseros-claw-sdk /tmp/browseros-claw-sdk
COPY --from=server /out/source-digest /usr/share/neo-sdk/source-digest
RUN set -eu; bundled=$(neo-server-info path); install -m 0755 /tmp/browseros-claw-sdk "$bundled"; rm /tmp/browseros-claw-sdk; \
    [ "$(neo-server-info kind)" = claw-sdk ] || { echo "the installed server is not claw-sdk" >&2; exit 1; }

RUN userdel -r ubuntu && useradd -m -u 1000 -s /bin/bash neo

# One supervisor for both images: a policy, quit or port fix lands in packages/browseros/docker/neo once.
COPY --from=neo-docker openbox-rc.xml /etc/neo/openbox-rc.xml
COPY --from=neo-docker --chmod=0755 entrypoint.sh /usr/local/bin/neo-entrypoint
COPY --from=neo-docker --chmod=0755 healthcheck.sh /usr/local/bin/neo-healthcheck
COPY --from=neo-docker --chmod=0755 neo-quit.py /usr/local/bin/neo-quit
COPY crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md crates/neo-sdk/neo-playwright/third_party/playwright/LICENSE crates/neo-sdk/neo-playwright/third_party/playwright/NOTICE /usr/share/doc/neo-sdk/playwright/

# What this image is made of, for hosts and for accept.sh: the source commit, the browser, and
# the server binary as installed (its sha256, the version compiled into it, and the digest of the
# sources it was compiled from, which must equal the digest of the sources this build copied;
# build.sh checks the binary against it before it tags the image). The commit's ARGs
# are declared here, not at the top: every RUN after an ARG misses the cache when its value
# changes, which would reinstall the packages and download the browser for every commit.
ARG NEO_SDK_REVISION=unknown
ARG NEO_SDK_DIRTY=true
ARG NEO_SDK_VERSION=unknown
RUN set -eu; . /usr/share/neo-sdk/browser.lock; server=$(neo-server-info path); \
    jq -n --arg version "$NEO_SDK_VERSION" --arg revision "$NEO_SDK_REVISION" --argjson dirty "$NEO_SDK_DIRTY" --arg arch "$TARGETARCH" \
      --arg browser "$NEO_BROWSER_VERSION" --arg deb "$(dpkg-query -W -f '${Version}' browserclaw)" \
      --arg server_sha256 "$(sha256sum "$server" | cut -d' ' -f1)" \
      --arg marker "$(LC_ALL=C grep -aoE 'browseros-claw-sdk-version=[^;]*' "$server" | head -1 | cut -d= -f2)" \
      --arg sources "$(cat /usr/share/neo-sdk/source-digest)" --arg path "$server" \
      --arg built_from "$(LC_ALL=C grep -aoE 'browseros-claw-sdk-source=[0-9a-f]{64}' "$server" | head -1 | cut -d= -f2)" \
      '{version: $version, revision: $revision, dirty: $dirty, arch: $arch, source_digest: $sources, browser: {version: $browser, deb: $deb}, server: {binary: "browseros-claw-sdk", path: $path, version: $marker, sha256: $server_sha256, source_digest: $built_from, toolchain: "rust 1.98.0, ziglang 0.15.2, cargo-zigbuild 0.23.0, glibc 2.17"}}' \
      > /usr/share/neo-sdk/provenance.json; \
    [ "$(jq -r '.server.source_digest == .source_digest' /usr/share/neo-sdk/provenance.json)" = true ] \
      || { echo "the installed server was not compiled from this build's sources" >&2; exit 1; }

# The host contract's defaults (README.md). NEO_MCP_REMOTE=false keeps the browser's two
# unauthenticated MCP proxies, 9010 and 9011, to loopback peers: the host drives Neo from
# inside the container. The CAPTCHA key paths are read if the host puts files there.
ENV NEO_SCREEN=1600x1000 \
    NEO_DATA_DIR=/data \
    NEO_DOWNLOAD_DIR="" \
    BROWSERCLAW_HOSTED_MODE=1 \
    NEO_SANDBOX=false \
    NEO_SOFTWARE_GRAPHICS=true \
    NEO_DISABLE_BROWSEROS_EXTENSIONS=true \
    NEO_ARGS="" \
    NEO_MCP_REMOTE=false \
    NEO_HOST_GID="" \
    BROWSEROS_CAPSOLVER_API_KEY_FILE=/home/neo/.config/browseros/captcha/capsolver-api-key \
    BROWSEROS_2CAPTCHA_API_KEY_FILE=/home/neo/.config/browseros/captcha/2captcha-api-key \
    PUID=1000 \
    PGID=1000

LABEL org.opencontainers.image.title="BrowserOS Neo SDK" \
      org.opencontainers.image.source="https://github.com/browseros-ai/BrowserOS" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.revision="$NEO_SDK_REVISION" \
      org.opencontainers.image.version="$NEO_SDK_VERSION"

# Only 9010 is declared. 9011 listens too and must be firewalled with it (README "Ports").
EXPOSE 9010
HEALTHCHECK --interval=5s --timeout=5s --start-period=90s --retries=3 CMD ["/usr/local/bin/neo-healthcheck"]
# tini reaps orphans and forwards SIGTERM to the entrypoint, which quits the browser cleanly.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/neo-entrypoint"]

# Keys baked in, only when build.sh is given key files. They are part of the image and readable
# by anyone who has it; BuildKit keeps them out of build arguments and logs only. build.sh
# rebuilds this stage without its cache every time, so a rotated or removed key never survives
# in a cached layer.
FROM neo-sdk-runtime AS neo-sdk-keyed
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
LABEL com.browseros.neo-sdk.captcha-keys="baked"

# The default target. It never mounts a secret, so no key can reach it from any cache.
FROM neo-sdk-runtime AS neo-sdk
