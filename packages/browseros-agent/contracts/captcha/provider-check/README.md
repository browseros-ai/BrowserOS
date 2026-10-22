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
| CapMonster | `BROWSEROS_CAPMONSTER_API_KEY_FILE` | `~/.config/browseros/captcha/capmonster-api-key` |

`BROWSEROS_CAPSOLVER_API_URL` and `BROWSEROS_CAPMONSTER_API_URL` override the respective defaults
`https://api.capsolver.com` and `https://api.capmonster.cloud` for local tests.
These names follow the solver contract; unprefixed names are ignored.
Each HTTP request has a 15-second timeout. CapMonster polls every three seconds
for at most 60 seconds; pending requests share that remaining time budget.

A positive balance permits one image-classification submission per vendor.
There are no submission retries. Zero balances and `--balance-only` never create
tasks. The task counter conservatively counts submissions, including ones whose
response fails: a lost response may still be billed. Tile indices are zero-based.
All CLI output passes through key redaction, including vendor and usage errors.

Exit status is **0** only when both vendors pass their balance and task checks;
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
and [CapMonster ComplexImageTask](https://docs.capmonster.cloud/docs/captchas/recaptcha-click/).
