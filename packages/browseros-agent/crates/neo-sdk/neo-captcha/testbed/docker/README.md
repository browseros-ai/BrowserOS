# Docker CAPTCHA E2E

The test bed against the Neo SDK image (`crates/neo-sdk/neo-server/docker`): the released neo
browser and this checkout's `browseros-claw-sdk`, in a throwaway container. From
`packages/browseros-agent`:

```sh
bun run test:captcha-docker-e2e -- --keys placeholder
bun run test:captcha-docker-e2e -- --keys placeholder --pages spotting
bun run test:captcha-docker-e2e -- --keys none --expect-tool-missing --only no-captcha
bun run test:captcha-docker-e2e -- --image neo-sdk:<tag> --keep

# Paid run: real CapSolver and 2Captcha keys, at most 12 paid tasks unless --paid-task-limit
bun run test:captcha-docker-e2e -- --keys real --paid --out ../../.llm/captcha-paid
```

Needs Docker with Linux support for the engine's architecture, jq, and Bun. No Chromium build.

1. **Image.** Builds this checkout's image with `build.sh --tag neo-captcha-e2e:<run>`, which tags
   it only once its server is proven to be this checkout's. `--image TAG` uses an existing SDK
   image instead.
2. **Identity, before starting it.** `neo-server-info kind` must say `claw-sdk` and
   `/usr/share/neo-sdk/provenance.json` must name `browseros-claw-sdk`; a built image must also carry
   this checkout's revision. A claw-server image (the production Neo image) is refused.
3. **Container.** Its own Docker network, `NEO_MCP_REMOTE=true` with 9010 published on
   `127.0.0.1` only, a fresh `/data`, and `host.docker.internal` and `captcha-frame.test` mapped to
   the host, where the fixtures listen on two sites. The harness owns these addresses and refuses
   `--mcp-url`, `--server` and fixture-address overrides.
4. **Running server.** After the healthcheck, and again after the pages, it hashes every process
   running the bundled server path (`/proc/PID/exe`, as user `neo`, since root cannot read another
   uid's executable) against `provenance.json`'s sha256.
5. **Pages.** `bun run test:captcha-e2e -- --mcp-url <container>/mcp --pages all`, plus every other
   flag given here, such as `--only`, `--pages spotting` or `--expect-tool-missing`.

Keys are mounted read-only under `/run/neo-keys/`, outside `/home/neo` as the image's README
requires, and `BROWSEROS_CAPSOLVER_API_KEY_FILE` and `BROWSEROS_2CAPTCHA_API_KEY_FILE` are always
set, blank for a provider without a key, so a key baked into an image is never read. `placeholder`
mounts one fake CapSolver file; `none` mounts nothing, so `neo.solveCaptcha` is unlisted and the
runner stops with a setup blocker unless `--expect-tool-missing` is given; `real` mounts the files
named by those variables, else `~/.config/browseros/captcha/{capsolver,2captcha}-api-key`, and sets
`BROWSEROS_CAPTCHA_PAID_TASK_LIMIT` (default 12). Key contents are never read, copied, baked into
an image or passed as build arguments.

Results and `proof.json` (image, provenance, running-server hashes, tools, step timings) go to
`../../.llm/captcha-docker-results` unless `--out` is set. `--keep` leaves the container, its
network and its mounts for inspection and prints the cleanup commands; otherwise all three are
removed on exit. `--clean` also removes the image this run built. A setup blocker from the runner
exits 2; any other failure exits 1.
