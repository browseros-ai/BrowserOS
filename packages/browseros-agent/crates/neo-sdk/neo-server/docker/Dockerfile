# syntax=docker/dockerfile:1
#
# The BrowserOS Neo SDK image: the released neo browser plus this commit's SDK server
# (browseros-claw-sdk, one MCP tool: `run`), for hosts that build on it or run it alone.
# README.md beside this file is the host contract. Build it with build.sh, from one
# context, packages/browseros-agent, filtered by Dockerfile.dockerignore. The image owns its
# browser supervisor (supervisor/ beside this file); the default Neo image, packages/browseros/
# docker/neo with claw-server, is production's and shares nothing with it.
#
#   stage server          (build machine's arch)  workspace minus supervisor/ and viewer ─▶ server
#   stage slimviewer-check (target arch, CI only) Rust tests for OpenH264 and x264
#   stage slimviewer-build (target arch)          OpenH264-only binary, web assets and notices
#   stage neo-sdk-runtime (target arch)           ubuntu + pinned browser .deb + server
#                                                 + slimviewer + supervisor/ + provenance
#   stage neo-sdk-keyed   neo-sdk-runtime + CAPTCHA keys from BuildKit secrets (build.sh --*-key-file)
#   stage neo-sdk         neo-sdk-runtime as is: keyless, the default target

ARG UBUNTU_IMAGE=ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852
ARG RUST_IMAGE=rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922

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
# The supervisor and standalone viewer have their own stages or runtime layers. Neither
# belongs in the server's build cache key or its compiled-in source digest.
COPY --exclude=crates/neo-sdk/neo-server/docker/supervisor --exclude=crates/neo-sdk/neo-slimviewer . .
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

# OpenH264's makefile and assembler run on the target CPU, as does the Rust linker.
FROM ${RUST_IMAGE} AS slimviewer-toolchain
RUN apt-get update && apt-get install -y --no-install-recommends g++ nasm && rm -rf /var/lib/apt/lists/*
ARG SOURCE_DATE_EPOCH=0
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    CARGO_INCREMENTAL=0 \
    RUSTFLAGS="--remap-path-prefix=/src=. --remap-path-prefix=/usr/local/cargo=/cargo --remap-path-prefix=/usr/local/rustup=/rust" \
    CFLAGS="-ffile-prefix-map=/src=. -ffile-prefix-map=/usr/local/cargo=/cargo" \
    CXXFLAGS="-ffile-prefix-map=/src=. -ffile-prefix-map=/usr/local/cargo=/cargo" \
    OPENH264_ROOT=/opt/openh264
COPY crates/neo-sdk/neo-slimviewer/csrc/build-openh264.sh /tmp/build-openh264.sh
RUN sh /tmp/build-openh264.sh /opt/openh264
WORKDIR /src
COPY --exclude=tests --exclude=web crates/neo-sdk/neo-slimviewer/ .
ARG TARGETARCH
ARG NEO_SDK_CACHE_SCOPE=
ARG NEO_SDK_BUILD_JOBS=

# Only the explicit check target installs x264 or compiles tests.
FROM slimviewer-toolchain AS slimviewer-check
RUN apt-get update && apt-get install -y --no-install-recommends libx264-dev && rm -rf /var/lib/apt/lists/* \
 && rustup component add clippy rustfmt
COPY crates/neo-sdk/neo-slimviewer/tests/ tests/
RUN --mount=type=cache,id=neo-slimviewer-registry-${TARGETARCH},target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=neo-slimviewer-target-${TARGETARCH}${NEO_SDK_CACHE_SCOPE},target=/src/target,sharing=locked \
    set -eu; \
    if [ -n "$NEO_SDK_BUILD_JOBS" ]; then export CARGO_BUILD_JOBS="$NEO_SDK_BUILD_JOBS"; fi; \
    cargo fmt --check; \
    cargo clean --locked --profile dev -p "$(cargo pkgid --locked)"; \
    cargo clippy --locked --all-targets -- -D warnings; \
    cargo test --locked; \
    cargo clippy --locked --all-targets --no-default-features --features x264 -- -D warnings; \
    cargo test --locked --no-default-features --features x264; \
    cargo clippy --locked --all-targets --all-features -- -D warnings; \
    cargo test --locked --all-features

FROM slimviewer-toolchain AS slimviewer-build
# Cargo uses mtimes. Rebuild the local crate and its C++ shim even for an older checkout.
RUN --mount=type=cache,id=neo-slimviewer-registry-${TARGETARCH},target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=neo-slimviewer-target-${TARGETARCH}${NEO_SDK_CACHE_SCOPE},target=/src/target,sharing=locked \
    set -eu; \
    if [ -n "$NEO_SDK_BUILD_JOBS" ]; then export CARGO_BUILD_JOBS="$NEO_SDK_BUILD_JOBS"; fi; \
    cargo clean --locked --release -p "$(cargo pkgid --locked)"; \
    cargo build --release --locked --no-default-features --features openh264; \
    binary=target/release/slimviewer; \
    ldd "$binary" > /tmp/linked-libraries; \
    if grep -E 'x264|not found' /tmp/linked-libraries; then exit 1; fi; \
    nm -D "$binary" > /tmp/dynamic-symbols; \
    if grep -E '[[:space:]](x264_|sv_x264_)' /tmp/dynamic-symbols; then exit 1; fi; \
    if "$binary" --encoder x264 > /tmp/x264-rejection 2>&1; then exit 1; fi; \
    grep -F 'encoder x264 is not built in' /tmp/x264-rejection; \
    strip --strip-all "$binary"; "$binary" --help; \
    install -D -m 0755 "$binary" /out/usr/local/bin/slimviewer

# Audit every dependency in the lockfile, including build-only and foreign-target crates.
# Ubuntu ships the copyright notices for the runtime's dynamically linked system libraries.
RUN --mount=type=cache,id=neo-slimviewer-registry-${TARGETARCH},target=/usr/local/cargo/registry,sharing=locked <<'AUDIT'
set -eu
cargo metadata --locked --format-version 1 > /tmp/cargo-metadata.json
python3 - <<'PY'
import json
import pathlib
import subprocess

metadata = json.loads(pathlib.Path('/tmp/cargo-metadata.json').read_text())
# Reviewed SPDX alternatives, selecting permissive terms. Unknown expressions,
# GPL/AGPL-only crates and unreviewed local dependencies fail the release build.
approved = {
    'MIT': 'MIT',
    'Apache-2.0': 'Apache-2.0',
    'MIT OR Apache-2.0': 'MIT',
    'Apache-2.0 OR MIT': 'MIT',
    'Unlicense OR MIT': 'MIT',
    'Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT': 'MIT',
    'MIT OR Apache-2.0 OR LGPL-2.1-or-later': 'MIT',
    '(MIT OR Apache-2.0) AND Unicode-3.0': 'MIT AND Unicode-3.0',
}
packages = [p for p in metadata['packages'] if p['id'] != metadata['resolve']['root']]
for package in packages:
    if package['source'] is None or package.get('license') not in approved:
        raise SystemExit(f"Unapproved crate licence: {package['name']} {package.get('license')}")
print(f'Crate licence audit: {len(packages)} packages; no GPL/AGPL licence selected.')
out = pathlib.Path('/out/usr/share/doc/slimviewer/THIRD-PARTY-LICENSES')
out.parent.mkdir(parents=True, exist_ok=True)

def notice_files(root):
    prefixes = ('LICENSE', 'LICENCE', 'COPYING', 'COPYRIGHT', 'NOTICE', 'UNLICENSE', 'AUTHORS')
    return sorted(p for p in root.rglob('*') if p.is_file()
                  and any(part.upper().startswith(prefixes) for part in p.relative_to(root).parts))

with out.open('w') as report:
    def section(title, files, root):
        if not files:
            raise SystemExit(f'Missing licence text/notice: {title}')
        report.write(f'\n{"=" * 72}\n{title}\n')
        for path in files:
            report.write(f'\n--- {path.relative_to(root)} ---\n')
            report.write(path.read_text())
            report.write('\n')

    report.write('Third-party notices for slimviewer\nGenerated from Cargo.lock and the pinned build environment.\n')
    report.write(f'Crate licence audit: {len(packages)} packages; selected terms below.\n')
    for package in sorted(packages, key=lambda p: (p['name'], p['version'])):
        root = pathlib.Path(package['manifest_path']).parent
        files = notice_files(root)
        if package.get('license_file'):
            explicit = root / package['license_file']
            if explicit not in files:
                files.append(explicit)
        section(f"{package['name']} {package['version']} — {package['license']}; selected: {approved[package['license']]}",
                sorted(files), root)

    root = pathlib.Path('/opt/openh264/share/doc/openh264')
    version, _ = (root / 'SOURCE').read_text().split()
    section(f'OpenH264 {version.removeprefix("v")} — BSD-2-Clause (built from source)',
            [root / 'SOURCE', root / 'LICENSE'], root)

    sysroot = pathlib.Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
    rust_docs = sysroot / 'share/doc/rust'
    rust_version = subprocess.check_output(['rustc', '--version'], text=True).split()[1]
    section(f'Rust standard library {rust_version}',
            [rust_docs / 'COPYRIGHT-library.html'] +
            sorted(p for p in (rust_docs / 'licenses').rglob('*') if p.is_file()), rust_docs)
PY
AUDIT

COPY --chmod=0644 crates/neo-sdk/neo-slimviewer/web/slimviewer.html crates/neo-sdk/neo-slimviewer/web/ui.js /out/usr/share/slimviewer/
# COPY --chmod also gives the folder it creates 0644, which no other user can enter; hosts read
# the page and adapter from here as any user.
RUN chmod 0755 /out/usr/share/slimviewer \
 && printf '%s\n' libc6 libgcc-s1 libstdc++6 > /out/usr/share/slimviewer/RUNTIME-DEPS \
 && find /out -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

FROM ${UBUNTU_IMAGE} AS neo-sdk-runtime
ARG TARGETARCH
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ENV DEBIAN_FRONTEND=noninteractive

# slimviewer connects directly to Xvnc. neo-quit uses python3 to flush browser state at shutdown.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl jq tini python3 \
      tigervnc-standalone-server tigervnc-tools openbox \
      libasound2t64 fonts-liberation fonts-noto-core fonts-noto-color-emoji fonts-wqy-zenhei fonts-ipafont-gothic \
 && rm -rf /var/lib/apt/lists/*

# The browser is pinned by version and per-architecture sha256 in browser.lock, so a bump is a
# reviewed one-file diff and a changed download fails the build. The download retries every
# failure (a CDN reset or a stream cut off mid-file failed image jobs before), each try from the
# start into the same file; the checksum after it still fails a short or wrong file, retried or not.
COPY crates/neo-sdk/neo-server/docker/browser.lock /usr/share/neo-sdk/browser.lock
RUN set -eu; . /usr/share/neo-sdk/browser.lock; \
    case "$TARGETARCH" in \
      amd64) url=$NEO_BROWSER_URL_AMD64 sha=$NEO_BROWSER_SHA256_AMD64 ;; \
      arm64) url=$NEO_BROWSER_URL_ARM64 sha=$NEO_BROWSER_SHA256_ARM64 ;; \
      *) echo "no browser for $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL --connect-timeout 30 --retry 5 --retry-delay 5 --retry-all-errors -o /tmp/neo.deb "$url"; \
    echo "$sha  /tmp/neo.deb" | sha256sum -c -; \
    apt-get update; apt-get install -y --no-install-recommends /tmp/neo.deb; \
    rm -rf /tmp/neo.deb /var/lib/apt/lists/*

# The server goes where the browser launches its bundled one from, and the build fails unless
# the installed binary's version marker says claw-sdk: the entrypoint picks the display, the host
# socket and the settings call from that marker at every start.
COPY --chmod=0755 crates/neo-sdk/neo-server/docker/supervisor/neo-server-info.sh /usr/local/bin/neo-server-info
COPY --from=server /out/browseros-claw-sdk /tmp/browseros-claw-sdk
COPY --from=server /out/source-digest /usr/share/neo-sdk/source-digest
RUN set -eu; bundled=$(neo-server-info path); install -m 0755 /tmp/browseros-claw-sdk "$bundled"; rm /tmp/browseros-claw-sdk; \
    [ "$(neo-server-info kind)" = claw-sdk ] || { echo "the installed server is not claw-sdk" >&2; exit 1; }

RUN userdel -r ubuntu && useradd -m -u 1000 -s /bin/bash neo

# The supervisor, the SDK image's own (supervisor/; it started as a copy of the default image's
# and is changed here only): the display, the browser, the host socket, the healthcheck, the quit.
COPY crates/neo-sdk/neo-server/docker/supervisor/openbox-rc.xml /etc/neo/openbox-rc.xml
COPY --chmod=0755 crates/neo-sdk/neo-server/docker/supervisor/entrypoint.sh /usr/local/bin/neo-entrypoint
COPY --chmod=0755 crates/neo-sdk/neo-server/docker/supervisor/healthcheck.sh /usr/local/bin/neo-healthcheck
COPY --chmod=0755 crates/neo-sdk/neo-server/docker/supervisor/neo-quit.py /usr/local/bin/neo-quit
COPY crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md crates/neo-sdk/neo-playwright/third_party/playwright/LICENSE crates/neo-sdk/neo-playwright/third_party/playwright/NOTICE /usr/share/doc/neo-sdk/playwright/

COPY --from=slimviewer-build /out/ /
RUN set -eu; while read -r package; do \
      [ "$(dpkg-query -W -f='${db:Status-Status}' "$package")" = installed ] \
        || { echo "missing slimviewer runtime package: $package" >&2; exit 1; }; \
    done < /usr/share/slimviewer/RUNTIME-DEPS

# What this image is made of, for hosts and for accept.sh: the source commit, the browser, and
# the server binary as installed (its sha256, the version compiled into it, and the digest of the
# sources it was compiled from, which must equal the digest of the sources this build copied;
# build.sh checks the binary against it before it tags the image), and the CAPTCHA keys it carries
# (`captchaKeys`: provider names only, never a value or a hash of one; empty here, filled in by the
# keyed target). The commit's ARGs
# are declared here, not at the top: every RUN after an ARG misses the cache when its value
# changes, which would reinstall the packages and download the browser for every commit.
ARG NEO_SDK_REVISION=unknown
ARG NEO_SDK_DIRTY=true
ARG NEO_SDK_VERSION=unknown
# Read the same OpenH264 pin that build.rs verified when it linked the binary.
RUN --mount=from=slimviewer-build,source=/opt/openh264/share/doc/openh264/SOURCE,target=/tmp/openh264-source \
    set -eu; . /usr/share/neo-sdk/browser.lock; server=$(neo-server-info path); \
    read -r openh264_version openh264_revision < /tmp/openh264-source; \
    jq -n --arg version "$NEO_SDK_VERSION" --arg revision "$NEO_SDK_REVISION" --argjson dirty "$NEO_SDK_DIRTY" --arg arch "$TARGETARCH" \
      --arg browser "$NEO_BROWSER_VERSION" --arg deb "$(dpkg-query -W -f '${Version}' browserclaw)" \
      --arg slimviewer_sha256 "$(sha256sum /usr/local/bin/slimviewer | cut -d' ' -f1)" \
      --arg openh264_version "${openh264_version#v}" --arg openh264_revision "$openh264_revision" \
      --arg server_sha256 "$(sha256sum "$server" | cut -d' ' -f1)" \
      --arg marker "$(LC_ALL=C grep -aoE 'browseros-claw-sdk-version=[^;]*' "$server" | head -1 | cut -d= -f2)" \
      --arg sources "$(cat /usr/share/neo-sdk/source-digest)" --arg path "$server" \
      --arg built_from "$(LC_ALL=C grep -aoE 'browseros-claw-sdk-source=[0-9a-f]{64}' "$server" | head -1 | cut -d= -f2)" \
      '{version: $version, revision: $revision, dirty: $dirty, arch: $arch, source_digest: $sources, \
        browser: {version: $browser, deb: $deb}, \
        server: {binary: "browseros-claw-sdk", path: $path, version: $marker, sha256: $server_sha256, \
                 source_digest: $built_from, toolchain: "rust 1.98.0, ziglang 0.15.2, cargo-zigbuild 0.23.0, glibc 2.17"}, \
        slimviewer: {sha256: $slimviewer_sha256, encoders: ["openh264"], \
                     openh264: {version: $openh264_version, revision: $openh264_revision}}, captchaKeys: []}' \
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
      io.browseros.slimviewer.encoders="openh264" \
      org.opencontainers.image.source="https://github.com/browseros-ai/BrowserOS" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.revision="$NEO_SDK_REVISION" \
      org.opencontainers.image.version="$NEO_SDK_VERSION"

# Only 9010 is declared. 9011 listens too and must be firewalled with it (README "Ports").
EXPOSE 9010
HEALTHCHECK --interval=5s --timeout=5s --start-period=90s --retries=3 CMD ["/usr/local/bin/neo-healthcheck"]
# tini reaps orphans and forwards SIGTERM to the entrypoint, which quits the browser cleanly.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/neo-entrypoint"]

# Keys baked in, only when build.sh is given key files: the published image, built by
# neo-sdk-image.yml from the neo-captcha environment's secrets. They are part of the image and
# readable by anyone who has it; BuildKit keeps them out of build arguments and logs only.
# provenance.json's captchaKeys names the keys this stage installed, so accept.sh checks the
# files against what the image says it carries. build.sh rebuilds this stage without its cache
# every time, so a rotated or removed key never survives in a cached layer.
FROM neo-sdk-runtime AS neo-sdk-keyed
RUN --mount=type=secret,id=capsolver_api_key,required=false \
    --mount=type=secret,id=twocaptcha_api_key,required=false \
    set -eu; \
    destination=/home/neo/.config/browseros/captcha; \
    install -d -m 0700 -o neo -g neo "$destination"; \
    keys=""; \
    if [ -s /run/secrets/capsolver_api_key ]; then \
      install -m 0400 -o neo -g neo /run/secrets/capsolver_api_key "$destination/capsolver-api-key"; \
      keys="$keys capsolver"; \
    fi; \
    if [ -s /run/secrets/twocaptcha_api_key ]; then \
      install -m 0400 -o neo -g neo /run/secrets/twocaptcha_api_key "$destination/2captcha-api-key"; \
      keys="$keys 2captcha"; \
    fi; \
    jq --arg keys "$keys" '.captchaKeys = ($keys | split(" ") | map(select(. != "")))' \
      /usr/share/neo-sdk/provenance.json > /tmp/provenance.json; \
    install -m 0644 /tmp/provenance.json /usr/share/neo-sdk/provenance.json; \
    rm /tmp/provenance.json
LABEL com.browseros.neo-sdk.captcha-keys="baked"

# The default target. It never mounts a secret, so no key can reach it from any cache.
FROM neo-sdk-runtime AS neo-sdk
