# BrowserOS CLI

## Unreleased

- Fix `open` and `window create` failing with an `unknown field "hidden"` schema error: the server retired the `hidden` input, so the CLI no longer sends it. Also restores the `page` field in `open --json` output. Removes the `--hidden` flag from `open` and `window create`.
- Add `--llm-txt`: prints a concise agent usage guide for the CLI to stdout (`browseros-cli --llm-txt`).
