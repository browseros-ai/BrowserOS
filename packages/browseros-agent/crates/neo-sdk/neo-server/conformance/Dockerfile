# Linux is authoritative. Toolchains and the multi-arch BrowserOS release are explicit inputs;
# the final image contains the freshly built SDK, never the server bundled with BrowserOS.
ARG NEO_IMAGE=ghcr.io/browseros-ai/neo@sha256:6b8939714b29538cb52765397cf14cbfbf71933732de19a350de1d268c51644d
FROM node:24.22.2-bookworm-slim AS node
FROM oven/bun:1.4.2 AS bun
FROM rust:1.98.0-bookworm AS builder
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun
WORKDIR /repo/packages/browseros-agent
COPY . /repo
ARG SERVER_PACKAGE=neo-server
RUN bun install --frozen-lockfile --ignore-scripts
# Examples enable dev-dependencies (including the crate's test-support feature). Build the
# production binary separately and last so its feature set matches a normal server build.
RUN CARGO_BUILD_JOBS=4 cargo build --locked -p ${SERVER_PACKAGE} --example conformance_compile && \
    CARGO_BUILD_JOBS=4 cargo build --locked -p ${SERVER_PACKAGE} --bin browseros-claw-sdk
FROM ${NEO_IMAGE} AS rig
USER root
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates && rm -rf /var/lib/apt/lists/*
# The Neo entrypoint normally fixes home ownership before dropping privileges. This rig
# bypasses it, so Crashpad needs the image's root-owned .config made writable here.
RUN chown -R 1000:1000 /home/neo
COPY --from=node /usr/local/bin/node /usr/local/bin/node
ENTRYPOINT ["/usr/local/bin/node"]
FROM rig AS conformance
# Copy only runtime inputs. Rust intermediates and unrelated app workspaces do not belong in
# the runner image; retaining them makes artifacts unnecessarily large.
COPY --from=builder /repo/packages/browseros-agent/node_modules /repo/packages/browseros-agent/node_modules
COPY --from=builder /repo/packages/browseros-agent/crates/neo-sdk /repo/packages/browseros-agent/crates/neo-sdk
COPY --from=builder /repo/packages/browseros-agent/target/debug/browseros-claw-sdk /repo/packages/browseros-agent/target/debug/browseros-claw-sdk
COPY --from=builder /repo/packages/browseros-agent/target/debug/examples/conformance_compile /repo/packages/browseros-agent/target/debug/examples/conformance_compile
ENV NEO_BINARY=/repo/packages/browseros-agent/target/debug/browseros-claw-sdk \
    BROWSEROS_APP=/usr/bin/browserclaw PLAYWRIGHT_SRC=/playwright CONFORMANCE_OUTPUT=/results
WORKDIR /repo/packages/browseros-agent
ARG SOURCE_SHA
ARG NEO_IMAGE
ENV NEO_GIT_SHA=${SOURCE_SHA} BROWSEROS_ARTIFACT=${NEO_IMAGE}
USER 1000:1000
CMD ["crates/neo-sdk/neo-server/conformance/conformance.mjs", "run"]
