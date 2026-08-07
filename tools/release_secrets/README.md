# Release Secrets Sync

`sync.py` syncs the release-workflow GitHub secrets from a local dotenv file to
repo-level secrets on `browseros-ai/BrowserOS`.

The tool is intentionally allowlist-only. It does not upload every key in the
dotenv file, and it never prints secret values. Apply mode sends values to
`gh secret set` over stdin.

```bash
tools/release_secrets/sync.py --env-file .env.production --dry-run

tools/release_secrets/sync.py --env-file .env.production --apply

tools/release_secrets/sync.py --check
```

When syncing from an operator checkout outside this repo, pass that file with
`--env-file /path/to/.env.production`.

`--check` scans the release workflow secret references and compares names
against `gh secret list`. Known non-dotenv names such as `GITHUB_TOKEN`,
`GH_TOKEN`, and the macOS certificate P12/password/keychain secrets are reported
separately from required missing repo secrets.
