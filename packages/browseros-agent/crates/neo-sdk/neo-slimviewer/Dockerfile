# syntax=docker/dockerfile:1
# Build context: this crate. The SDK imports only the files in the final stage.
# Keep RUST_IMAGE in sync with ../neo-server/docker/Dockerfile.
ARG RUST_IMAGE=rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922

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
COPY csrc/build-openh264.sh /tmp/build-openh264.sh
RUN sh /tmp/build-openh264.sh /opt/openh264
WORKDIR /src
COPY --exclude=tests --exclude=web . .
ARG TARGETARCH
ARG NEO_SDK_CACHE_SCOPE=
ARG NEO_SDK_BUILD_JOBS=

# Only the explicit check target installs x264 or compiles tests.
FROM slimviewer-toolchain AS slimviewer-check
RUN apt-get update && apt-get install -y --no-install-recommends libx264-dev && rm -rf /var/lib/apt/lists/* \
 && rustup component add clippy rustfmt
COPY tests/ tests/
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

COPY --chmod=0644 web/slimviewer.html web/ui.js /out/usr/share/slimviewer/
# COPY --chmod also gives the folder it creates 0644, which no other user can enter; hosts read
# the page and adapter from here as any user.
RUN chmod 0755 /out/usr/share/slimviewer \
 && printf '%s\n' libc6 libgcc-s1 libstdc++6 > /out/usr/share/slimviewer/RUNTIME-DEPS \
 && install -m 0644 "$OPENH264_ROOT/share/doc/openh264/SOURCE" /out/usr/share/doc/slimviewer/OPENH264-SOURCE \
 && find /out -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

FROM scratch
COPY --from=slimviewer-build /out/ /
