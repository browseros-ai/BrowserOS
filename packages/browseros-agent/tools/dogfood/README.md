# browseros-dogfood

Internal CLI for running BrowserOS neo from an alpha checkout against a copy of your BrowserOS profile.

## What it does

`browseros-dogfood` lets the team alpha test the latest dev branch of BrowserOS neo with the least possible setup.

- You point it at a BrowserOS repo clone used for alpha dogfooding.
- It tracks a configured branch for that clone and switches to it before builds and update commands.
- It imports your normal BrowserOS profile into a separate dogfood profile.
- It keeps dogfood state separate from your normal app state.
- It starts the BrowserOS neo WXT app and claw-server against the installed BrowserOS neo app. If BrowserOS neo is not installed, it uses the BrowserOS app instead.
- It does not pull on `start`. You choose when to update the checkout.

## Requirements

- macOS.
- Go.
- Bun.
- Rust and Cargo, to build claw-server.
- BrowserOS neo installed at `/Applications/BrowserOS neo.app`, or BrowserOS at `/Applications/BrowserOS.app` as the fallback.
- A separate BrowserOS monorepo checkout for alpha dogfood.

## Install

From the BrowserOS monorepo root:

```bash
cd packages/browseros-agent/tools/dogfood
make install
```

This installs `browseros-dogfood` globally on your machine.

Check the binary:

```bash
browseros-dogfood --help
```

## First-time setup

Run:

```bash
browseros-dogfood init
```

`init` asks for:

- `Repo path`: the full path to the root BrowserOS git repo clone.
- `Branch`: the branch dogfood should track. It defaults to the selected repo's current branch, or `main`.
- `BrowserOS binary`: defaults to `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`. With that default, `start` launches BrowserOS neo when it is installed.
- `Source profile`: your main installed BrowserOS profile.

Use a separate clone for the repo path. `browseros-dogfood` runs alpha dogfood builds from this clone, so ideally it is not the checkout you use for everyday dev work. Give the full root repo path, for example `/Users/you/code/browseros-alpha`.

If you have more than one BrowserOS profile, `init` reads them and shows their real names. Pick your main profile, the one with your data, so alpha dogfood starts with the right imported profile.

## Daily use

```bash
browseros-dogfood start
```

`start` runs in your terminal. Press `Ctrl+C` to stop the environment.

To run it in the background instead:

```bash
browseros-dogfood start-background
```

`start-background` keeps running after the command returns. Use the CLI to manage it:

```bash
browseros-dogfood status
browseros-dogfood pull
browseros-dogfood restart
browseros-dogfood restart --pull
browseros-dogfood logs
browseros-dogfood logs tail
browseros-dogfood stop
```

- `start` switches a clean checkout to the configured branch before building. It does not pull.
- `pull` switches to the configured branch and updates the configured repo for the next `start`.
- `restart --pull` switches to the configured branch, updates the configured repo, rebuilds, and restarts when new changes land upstream.
- `logs` prints the log file paths. `logs tail` follows the background daemon, claw-app, and claw-server logs.
- `--claw` (`-c`) is still accepted and does nothing, so older commands such as `browseros-dogfood --claw start` keep working.

## State and profile safety

`browseros-dogfood` keeps alpha dogfood separate from your normal BrowserOS setup:

- Dogfood state lives under `~/.browseros-claw-dogfood`.
- The imported dogfood profile lives under `~/.config/browseros-dogfood/claw/profile`.
- Your installed BrowserOS profile is only the source of the import. Alpha dogfood does not run in it.
- Installed extensions, extension-specific settings and state, and extension-owned IndexedDB data are copied, so dogfood sessions keep an extension setup close to your normal profile.
- Cache and broad site storage directories are not copied.

To re-import your main profile:

```bash
browseros-dogfood start --refresh-profile
```

If BrowserOS appears to be using the source profile during import, the CLI asks you to quit BrowserOS and press Enter before it copies. Type `continue` instead if the lock files are stale and you want to import anyway.

## Config

```bash
browseros-dogfood config edit
```

The config lives at `~/.config/browseros-dogfood/config.yaml`. Most people only edit it to change the alpha repo clone, the tracked branch, or the ports.

The claw-app launcher passes `--browseros-dock-icon=dev` to the browser, so dogfood sessions show the dev Dock icon.
