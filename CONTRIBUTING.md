# Contributing to BrowserOS neo and BrowserOS

Thanks for being here. Whether you are fixing a bug, building a feature, improving docs, or just poking around, we are glad to have you.

Both browsers ship from this one repo, and there are four places you can work. Pick the one that matches what you want to change.

## Pick your path

| Path | You would work on | Stack | Cost to set up |
|---|---|---|---|
| **[BrowserOS neo](packages/browseros-agent/CONTRIBUTING.md)** | The cockpit new tab, the MCP surface agents connect to, session replay | TypeScript, React, Rust | ~15 minutes |
| **[BrowserOS](packages/browseros-agent/CONTRIBUTING.BrowserOS.md)** | The side panel chat, the agent loop, scheduled tasks, settings | TypeScript, React, Bun | ~15 minutes |
| **CLI** | Driving BrowserOS from a terminal or a coding agent | Go | ~5 minutes |
| **Browser** | Chromium patches, the build system, platform features | C++, Python | ~100GB disk, hours |

Most contributors start with BrowserOS neo or BrowserOS. Both live in `packages/browseros-agent` and share one toolchain, so setting up for one gets you most of the way to the other.

## Before you start

**Use Bun.** It is the only supported package manager and runtime for the agent monorepo. `packages/browseros-agent/package.json` pins the version and rejects the alternatives outright:

```json
"packageManager": "bun@<pinned>",
"engines": {
  "bun": "<pinned>",
  "node": "please-use-bun",
  "npm": "please-use-bun",
  "yarn": "please-use-bun",
  "pnpm": "please-use-bun"
}
```

Install it with `curl -fsSL https://bun.sh/install | bash`, or `brew install oven-sh/bun/bun`. CI installs the exact pinned version by reading that same `package.json`, so matching it locally keeps you on the same dependency resolution. Check yours with `bun --version`.

The per-path guides list what else each one needs.

## Browser development

Building the Chromium fork is a different kind of work from everything above. Only go here if you are changing the browser itself rather than what runs inside it.

**You will need** roughly 100GB of free disk for the Chromium source, 16GB or more of RAM, Python 3.12+ with [uv](https://docs.astral.sh/uv/), and your platform's toolchain: Xcode command line tools on macOS, `build-essential` on Linux, or Visual Studio Build Tools on Windows.

**Get the Chromium source first.** Follow [Chromium: Get the Code](https://www.chromium.org/developers/how-tos/get-the-code/) for your platform. It sets up `depot_tools` and fetches the tree, which usually takes a few hours.

**Then build:**

```bash
cd packages/browseros
uv sync                                   # once
cp .env.example .env                      # once, then fill in what you need

# a debug build of either product
uv run browseros build --preset debug --product browseros   --chromium-src /path/to/chromium/src
uv run browseros build --preset debug --product browserclaw --chromium-src /path/to/chromium/src

# see exactly what a build would run, without running it
uv run browseros build --preset release --show-plan
```

Builds take one to three hours on modern hardware. `browseros build` produces one binary for one product on one platform; releasing is a separate workflow. For the full picture, read [`packages/browseros/bos_build/README.md`](packages/browseros/bos_build/README.md).

## Opening a pull request

- **Title in [Conventional Commits](https://www.conventionalcommits.org/) format.** A CI check enforces this.
- **Say what changed and why.** The why is the part reviewers cannot get from the diff.
- **Screenshots or a short video for anything visual.**
- **Link the issue** it closes, for example `Fixes #123`.

Run the checks before you push:

```bash
cd packages/browseros-agent
bun run check     # lint, typecheck and fallow in one pass
bun test          # the TypeScript suites
```

### Sign the CLA

On your first pull request a bot will ask you to sign the Contributor License Agreement. Read [CLA.md](CLA.md), then comment on your PR with exactly:

```
I have read the CLA Document and I hereby sign the CLA
```

The bot records it once and will not ask again.

## Other ways to help

You do not need to write code to be useful here.

- **Report a bug.** [Open an issue](https://github.com/browseros-ai/BrowserOS/issues/new/choose) with what you did, what you expected, what happened instead, and your OS and version. Screenshots or a recording help a lot.
- **Suggest a feature.** Start with the [issue chooser](https://github.com/browseros-ai/BrowserOS/issues/new/choose) or talk it through on [Discord](https://discord.gg/YKwjt5vuKr) first.
- **Improve the docs.** The site lives in [`docs/`](docs/) and is written in MDX. Fixing a wrong step you just hit is one of the most valuable contributions there is.
- **Test on your setup.** Different OS, different agent, unusual hardware. Edge cases are found by people who have them.

## Getting help

- **[Discord](https://discord.gg/YKwjt5vuKr)** and **[Slack](https://dub.sh/browserOS-slack)** for real-time questions
- **[GitHub Discussions](https://github.com/browseros-ai/BrowserOS/discussions)** for longer ones
- **[GitHub Issues](https://github.com/browseros-ai/BrowserOS/issues)** for bugs
- **Security issues:** do not open an issue. Follow [SECURITY.md](.github/SECURITY.md) and open a private advisory.

## License

By contributing, you agree that your contributions are licensed under AGPL-3.0.

---

Built with ❤️ from San Francisco
