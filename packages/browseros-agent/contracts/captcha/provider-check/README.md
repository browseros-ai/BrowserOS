# CAPTCHA provider check

From `packages/browseros-agent`:

```sh
bun run captcha:provider-check
bun run captcha:provider-check --balance-only
bun run captcha:provider-check --json
bun test contracts/captcha/provider-check
```

Reads and trims keys from these files:

| Vendor | File override | Default |
| --- | --- | --- |
| CapSolver | `BROWSEROS_CAPSOLVER_API_KEY_FILE` | `~/.config/browseros/captcha/capsolver-api-key` |
| 2Captcha (optional) | `BROWSEROS_2CAPTCHA_API_KEY_FILE` | `~/.config/browseros/captcha/2captcha-api-key` |

`BROWSEROS_CAPSOLVER_API_URL` and `BROWSEROS_2CAPTCHA_API_URL` override the respective defaults
`https://api.capsolver.com` and `https://api.2captcha.com` for local tests.
These names follow the solver contract; unprefixed names are ignored.
Each HTTP request has a 15-second timeout. 2Captcha polls every three seconds
for at most 60 seconds; pending requests share that remaining time budget.

CapSolver is required. A missing 2Captcha key file skips that optional vendor:

```text
2captcha    skipped balance=n/a    task=skipped  no key file; unverified: no key yet
```

An empty or unreadable key file is an error. A positive balance permits one
image-classification submission per configured vendor.
There are no submission retries. Zero balances and `--balance-only` never create
tasks. The task counter conservatively counts submissions, including ones whose
response fails: a lost response may still be billed. Tile indices are zero-based.
All CLI output passes through key redaction, including vendor and usage errors.

Exit status is **0** only when CapSolver and every configured vendor pass their balance and task checks;
**1** otherwise, including zero balances and `--balance-only`; **2** for usage
errors. A successful balance alone does not establish paid solving readiness.
JSON contains the same facts, including per-vendor task details and exit status.

## Sample provenance

`sample/grid.png` is an original synthetic 300×300 PNG (2,388 bytes), drawn for
this diagnostic using only rectangles, circles, and Python's standard-library
PNG encoding. It has nine tiles: traffic lights in tiles 0, 4, and 7, and simple
cars in the remaining tiles. No external images, screenshots, or Google content
were used. The image is provided under this repository's AGPL-3.0-or-later license.

This checks authorization, billing, and the paid task schema. The answer's
accuracy is irrelevant; it does not prove real CAPTCHA solving works.

Schemas: [CapSolver classification](https://docs.capsolver.com/en/guide/recognition/ReCaptchaClassification/)
and [2Captcha API v2 GridTask](https://2captcha.com/api-docs/grid).
The latter sends `body` (base64 PNG), `rows: 3`, `columns: 3`, and a `comment`
asking for traffic lights. It uses `createTask` then `getTaskResult`; the
one-based `solution.click` indices become zero-based report tiles.
Balances use [getBalance](https://2captcha.com/api-docs/get-balance).

Both providers passed the approved live API check on 2026-10-22: one task
each, returning tiles `[0,4,7]`. This does not verify solving on real demo pages.
Unit tests use local mock HTTP servers and fake keys only.
