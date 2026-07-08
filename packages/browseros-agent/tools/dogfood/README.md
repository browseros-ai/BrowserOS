# browseros-dogfood

Internal BrowserOS dogfooding CLI for running BrowserOS or BrowserClaw against a copied BrowserOS profile.

## What It Does

`browseros-dogfood` makes it easy for the team to alpha test the latest dev branch with the smallest possible effort.

High level:

- You point it at a BrowserOS repo clone used for alpha dogfooding.
- It tracks a configured branch for that clone and switches to it before builds and update commands.
- It imports your normal BrowserOS profile into a target-specific dogfood profile.
- It keeps BrowserOS and BrowserClaw state separate from your normal app state and from each other.
- For BrowserOS, it builds the local extension, starts the local server, and launches the installed BrowserOS app with the alpha Dock icon against them.
- For BrowserClaw, it starts the BrowserClaw WXT app and standalone Claw server against the installed BrowserClaw app, falling back to BrowserOS when BrowserClaw is not installed.
- It does not auto-pull on `start`; you choose when to update the checkout.

## Requirements

- macOS.
- Go.
- Bun.
- BrowserOS installed at `/Applications/BrowserOS.app`; BrowserClaw optionally installed at `/Applications/BrowserClaw.app`.
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

## First-Time Setup

Run one or both target setup commands:

```bash
browseros-dogfood --browseros init
browseros-dogfood --claw init
```

`init` asks for:

- `Repo path`: the full path to the root BrowserOS git repo clone.
- `Branch`: the branch dogfood should track. It defaults to the selected repo's current branch, or `main`.
- `BrowserOS binary`: defaults to `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`; Claw start resolves to BrowserClaw at launch time when available.
- `Source profile`: your main installed BrowserOS profile.

Use a separate clone for the repo path. This clone is what `browseros-dogfood` uses to run alpha dogfood builds, so ideally it is not the same checkout you use for actual dev work. Give the full root repo path, for example `/Users/you/code/browseros-alpha`.

If you have multiple BrowserOS profiles, `init` reads them and shows their real names. Pick your main profile, the one with your data, so alpha dogfood starts with the right imported profile.

## Daily Use

```bash
browseros-dogfood --browseros start
browseros-dogfood --claw start
```

`start` is sync: it runs in your terminal. Press `Ctrl+C` to cancel and stop the selected target environment.

For async mode:

```bash
browseros-dogfood --browseros start-background
browseros-dogfood --claw start-background
```

`start-background` keeps running after the command returns. Use the CLI to manage it:

```bash
browseros-dogfood --claw status
browseros-dogfood --claw pull
browseros-dogfood --claw restart
browseros-dogfood --claw restart --pull
browseros-dogfood --claw logs
browseros-dogfood --claw logs tail
browseros-dogfood --claw stop
```

- `start` switches a clean checkout to the configured branch before building. It still does not pull.
- `pull` switches to the configured branch and updates the configured repo for the next sync start.
- `restart --pull` switches to the configured branch, updates the configured repo, rebuilds, and restarts when new changes land upstream.
- `logs` prints log file paths; `logs tail` follows background dogfood, browser/app, and server logs.
- BrowserOS and BrowserClaw use separate locks, sockets, state files, profiles, and logs.

## State And Profile Safety

`browseros-dogfood` keeps alpha dogfood separate from normal BrowserOS:

- BrowserOS state, including the local server state and VM data, lives under `~/.browseros-dogfood`.
- BrowserClaw state lives under `~/.browseros-claw-dogfood`.
- The imported BrowserOS dogfood profile lives under `~/.config/browseros-dogfood/browseros/profile`.
- The imported BrowserClaw dogfood profile lives under `~/.config/browseros-dogfood/claw/profile`.
- Your installed BrowserOS profile is only used as the source import. It is not where alpha dogfood runs.
- Installed extensions, extension-specific settings/state, and extension-owned IndexedDB data are copied so dogfood sessions keep extension setup close to your normal profile.
- Cache and broad site storage directories are not copied.

To re-import your main profile:

```bash
browseros-dogfood --browseros start --refresh-profile
browseros-dogfood --claw start --refresh-profile
```

If BrowserOS appears to be using the source profile during import, the CLI asks you to quit BrowserOS and press Enter before copying. You can type `continue` if the lock files are stale and you want to import anyway.

## Config

```bash
browseros-dogfood config edit
```

Config lives at `~/.config/browseros-dogfood/config.yaml`. Most people should only need to edit it when changing the alpha repo clone, tracked branch, ports, or env values.

Browser launch passes `--browseros-dock-icon=alpha` so dogfood sessions are visually distinct in the Dock.
