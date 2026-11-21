# CAPTCHA fixtures

From `packages/browseros-agent`:

```sh
bun run captcha:fixtures
bun run captcha:fixtures --host 0.0.0.0 --public-host host.docker.internal --port 18080
bun run captcha:fixtures --host 0.0.0.0 --public-host host.docker.internal --frame-host captcha-frame.test
bun test crates/neo-sdk/neo-captcha/testbed/fixtures
```

The CLI prints fixture and demo URLs plus the frame origin. Both listeners bind
to `127.0.0.1` by default, with separate random ports in 10101–20202. Top URLs
use `127.0.0.1`; frame URLs use `localhost`. `--host` sets both bind addresses;
`--public-host` and `--frame-host` set their advertised hosts. `--port` and
`--frame-port` pin their ports (0 asks the OS to choose). Ctrl-C closes both.
Each listener serves the same pages. The server replaces `{{FRAME_ORIGIN}}` in
HTML with the second origin. Two ports alone are insufficient: use different
sites so Chromium can isolate the frame.

Each standalone page loads its vendor's real script. It mirrors widget state into `window.__captchaFixture` and `document.documentElement.dataset.captchaFixture`. Success records token length, while errors and expiry clear it. Rendering tracks the visible iframe, or Turnstile's visible host when the iframe has a closed shadow root. Submissions use real vendor siteverify endpoints and return `ACCEPTED` or `REJECTED: <codes>` with `data-captcha-verdict`. `verifyUrls` substitutes local endpoints in unit tests.

## Pages and public test keys

`neo.solveCaptcha(page)` returns `{ attempted, type, method, reason, message, durationMs, paidTasks, provider }` with no `verdict`; the verdicts below (`solved`, `token_applied`, `already_clear`, `unsolved`) are the test bed's own classification of the SDK's `reason`. Because Neo always solves a visible widget fresh instead of short-circuiting on a pre-existing token, a widget that already holds a token and has no checkbox to click requires a paid mint.

| Page | Proves | Sitekey |
|---|---|---|
| `recaptcha-v2.html` | Checkbox completion and server acceptance | `6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI` |
| `recaptcha-v2-stale.html` | Stale token in `g-recaptcha-response` on an unchecked box; clicking the checkbox must replace it for free (`checkbox_passed`, `attempted: true`, `paidTasks: 0`) | `6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI` |
| `recaptcha-v2-invisible-bind.html` | Button-bound invisible reCAPTCHA v2 (`not_blocking`, classified `already_clear`, `attempted: false`, `paidTasks: 0`) | `6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI` |
| `turnstile-pass.html` | Always-pass Turnstile widget; solve usually lands before the widget's token and passes free (`auto_passed`, `attempted: true`). If the widget wins the race, paid providers refuse this test key (`provider_error`, 0 paid tasks), which `providerRefusesKey: true` marks `UNVERIFIED` | `1x00000000000000000000AA` |
| `turnstile-stale.html` | Static failed-widget `.cf-turnstile` box ("Verification failed", no vendor script, no checkbox) holding a stale token on the interactive test key; Neo must mint a fresh paid token (`token_applied` / `solved`, `attempted: true`, `maxPaidTasks: 1`, `mayNeedPaid: true`; 2Captcha mints this key) | `3x00000000000000000000FF` |
| `turnstile-block.html` | Failure without a token | `2x00000000000000000000AB` |
| `turnstile-interactive.html` | Completion after checkbox interaction | `3x00000000000000000000FF` |
| `turnstile-stale-interactive.html` | Stale token in `cf-turnstile-response` while the interactive checkbox is unpassed; clicking the checkbox must replace it for free (`checkbox_passed`, `attempted: true`, `maxPaidTasks: 0`) | `3x00000000000000000000FF` |
| `turnstile-cf-marker.html` | Turnstile interactive checkbox beside Cloudflare's passive `_cf_chl_opt` bot-management marker | `3x00000000000000000000FF` |
| `cloudflare-wall.html` | Static Cloudflare full-page wall (`cloudflare_challenge`); never clears, so Neo reports `challenge_not_passed` (`unsolved`, `attempted: true`) | `3x00000000000000000000FF` |
| `no-captcha.html` | Ordinary form; always accepted (`no_captcha`, classified `already_clear`, `attempted: false`) | None |
| `hcaptcha.html` | hCaptcha detection; refused as `unsupported_type` (classified `unsolved`, `attempted: false`) | `10000000-ffff-ffff-ffff-000000000001` |
| `perimeterx-modal.html` | PerimeterX's press-and-hold check over a page, mimicked with no vendor script: `iframe#px-captcha-modal` and its `#px-captcha`; refused as `unsupported_type` (classified `unsolved`, `attempted: false`, type `unknown`, message naming PerimeterX) | None |
| `perimeterx-waiting.html` | The same frame inserted but not shown, as before it challenges (`no_captcha`, classified `already_clear`, `attempted: false`) | None |

### Stale-token fixtures

`turnstile-stale-interactive.html`, `turnstile-stale.html`, and `recaptcha-v2-stale.html` prove that Neo ignores an existing response token and writes a fresh one. `turnstile-stale-interactive.html` and `recaptcha-v2-stale.html` load their vendor's real script, wait until the widget is drawn and its response field exists, write an obvious fake constant (`staleToken` in `catalog.ts`) into the field, and prove a free checkbox click replaces it. `turnstile-stale.html` is the one stale page that loads **no vendor script and no live widget**: both paid providers refuse Cloudflare's always-pass key (`1x00000000000000000000AA`), whereas 2Captcha does mint the interactive test key (`3x00000000000000000000FF`), so `turnstile-stale.html` renders a static `.cf-turnstile` box showing "Verification failed" with `<input type="hidden" name="cf-turnstile-response">` holding `STALE-TOKEN-turnstile-do-not-accept` (modeling a widget that failed and left an old token in the form, with no checkbox to click). All three set `rendered: true` only once drawn with the stale token in the field, mirror `stale: true` and `replaced: false` (flipping `replaced` to `true` once the field's value differs, listening on `input` and `change`), and reject that exact constant on `POST /submit/<id>` with `REJECTED: stale-token` without consulting the vendor.

Public test secrets, verified against vendor documentation:

- [Google FAQ](https://developers.google.com/recaptcha/docs/faq#id-like-to-run-automated-tests-with-recaptcha.-what-should-i-do): `6LeIxAcTAAAAAGG-vFI1TnRWxMZNFuojJ4WifJWe`. The original brief's sitekey was invalid; the catalog uses Google's documented key, approved in the shared contract.
- [Cloudflare testing](https://developers.cloudflare.com/turnstile/troubleshooting/testing/): pass/interactive `1x0000000000000000000000000000000AA`; block `2x0000000000000000000000000000000AA`.
- [hCaptcha integration testing](https://docs.hcaptcha.com/#integration-testing-test-keys): `0x0000000000000000000000000000000000000000`.

## Browser checks

Checked with a temporary BrowserOS neo profile over CDP, without using the owner's browser. reCAPTCHA, Turnstile pass/interactive, hCaptcha and the ordinary form returned `ACCEPTED`. Turnstile block rendered, reported `600010`, produced no token, and returned `REJECTED: missing-input-response`. The hCaptcha test checkbox was completed manually; the solver still does not support it.

[hCaptcha recommends a named local host](https://docs.hcaptcha.com/#local-development). This build rendered and accepted the test token on both `127.0.0.1` and `captcha.test`. For environments enforcing its localhost/IP restrictions, serve with `--public-host captcha.test` and launch your test Chromium with `--host-resolver-rules="MAP captcha.test 127.0.0.1"`.

## Demo checks

Fetched with curl and opened in an isolated browser:

- [Google](https://www.google.com/recaptcha/api2/demo): real key; `#recaptcha-demo-submit`; acceptance text `Verification Success`.
- [Cloudflare](https://demo.turnstile.workers.dev/): public pass key, visible widget host, form posts to `/handler`; submitting its dummy token returned JSON with `"success":true`. `neo.solveCaptcha` usually lands before the widget's token and passes free (`mayNeedPaid: false`, `expectAttempted: true`); if the widget wins the race, paid providers refuse `1x00000000000000000000AA` (`provider_error`, 0 paid tasks), which `providerRefusesKey: true` marks `UNVERIFIED`.
- [hCaptcha](https://accounts.hcaptcha.com/demo): real key, rendered checkbox; no v1 acceptance check.
- [2Captcha reCAPTCHA v2](https://2captcha.com/demo/recaptcha-v2): live widget `#g-recaptcha`, real sitekey `6LfD3PIbAAAAAJs_eEHvoOl75_83eXSqpPSRFJ_u`; may need a paid solve.
- [2Captcha Turnstile](https://2captcha.com/demo/cloudflare-turnstile): live widget `#cf-turnstile`, public interactive **test key** `3x00000000000000000000FF`; no paid solve required. Readiness checks the visible host because its iframe is inside a closed shadow root.

Both 2Captcha demos were inspected in BrowserOS neo. Their `main form button[type="submit"]` checks the token with `/api/v1/captcha-demo/<slug>/verify`; the client renders the server's JSON with `"success": true` only after acceptance. The Turnstile test checkbox was completed and submitted: its real response contained `"success": true` and `"result_with_testing_key": true`. The reCAPTCHA acceptance path was read from the deployed client; no paid demo solve was attempted. Inspecting these public demos needs no provider key.

CapSolver's own-domain demo search found guides pointing to external demos, not a hosted demo. `demo.capsolver.com` did not resolve; no speculative entry was added. No third-party production site is included.

Provider readiness is checked separately by `bun run captcha:provider-check`.
Both providers passed the live API check; paid solving on these demo pages remains **unverified**. These fixtures never read provider API keys.

## Spotting fixtures

Run `bun run test:captcha-e2e -- --dev-watch --pages spotting --keys placeholder`.
Every row expects test-bed verdict `unsolved`, its exact type, and `paidTasks: 0`: it
proves detection, never a solve. The fake keys make the SDK answer
`challenge_not_passed` today (classified `unsolved`); a type refusal (`unsupported_type`) would pass too.
The runner records the reason as returned. These pages have no submit check.
They are also part of `fixtures` and `all`.

| Pages | Family | Real loader |
| --- | --- | --- |
| `geetest-v4.html`, `geetest-v4-frame.html`, `geetest-v4-shadow.html` | `geetest` | `static.geetest.com/v4/gt4.js` |
| `geetest-v3.html` | `geetest` | `static.geetest.com/static/js/gt.0.4.9.js` |
| `aws-waf.html`, `aws-waf-frame.html`, `aws-waf-shadow.html` | `aws_waf` | `9175c2fd4189.edge.captcha-sdk.awswaf.com/9175c2fd4189/jsapi.js` |
| `aws-waf-interstitial.html` | `aws_waf` | `41bcdd4fb3cb.610cd090.us-east-1.{token,captcha}.awswaf.com/.../{challenge,captcha}.js` |
| `funcaptcha.html`, `funcaptcha-frame.html`, `funcaptcha-shadow.html` | `arkose` | `client-api.arkoselabs.com/v2/00000000-0000-0000-0000-000000000000/api.js` |

GeeTest uses 32 zeroes for its IDs; Arkose uses the all-zero UUID. AWS uses
`FAKE_API_KEY_DO_NOT_USE` or clearly fake `gokuProps`. No key belongs to a site.
Each page attempts the real SDK and records `loadedScripts`, `fallback`, and
errors in its usual fixture state. A six-second fallback handles rejected fake
keys without inventing vendor globals or successful tokens. Readiness requires
every real loader to finish and rejects `script-load-error`, even if a fallback
widget is visible.

Frame parents contain only the cross-site iframe and a state relay; they load
no vendor script. The child posts its fixture state. The parent verifies the
message origin, source, ID, and family before mirroring readiness. Shadow pages
put the loader and widget under `attachShadow({mode: 'closed'})`; the root stays
local to the script. `host.shadowRoot` and page-level widget queries return null.
Only plain fixture state is exported to `window` and the dataset.

### Observed widget shapes (2026-10-26)

Inspected in the isolated dev-watch BrowserOS browser, without provider calls:

- **GeeTest v4:** the zero ID loads `gt4.js` and requests `gcaptcha4.geetest.com/load`,
  but draws nothing. The real [2Captcha v4 demo](https://2captcha.com/demo/geetest-v4)
  renders `div.geetest_holder_<suffix>.geetest_holder`, an `svg.geetest_btn_svg`,
  `div.geetest_btn_click`, and `div.geetest_content`. The fixture fallback copies
  that observed holder tree; the suffix is a vendor-generated style identifier.
- **GeeTest v3:** the real SDK renders its offline fallback even with the zero
  key: `div.geetest_holder.geetest_wind.geetest_fallback.geetest_ready`,
  `div.geetest_radar_btn`, and hidden `geetest_challenge`, `geetest_validate`,
  `geetest_seccode` inputs. Its additional script is `static/js/fullpage.0.0.0.js`.
- **AWS JS API:** the real loader creates `awswaf-captcha` (320px wide) with an
  **open** shadow root containing `#root`, `div.amzn-captcha-modal`, a form,
  `div.amzn-captcha-modal-title`, a 320×320 canvas, and
  `button#amzn-btn-verify-internal.btn.btn-primary`. It rendered a challenge with
  the fake API key on this endpoint; no acceptance was attempted. It also
  rendered inside the fixture's closed outer root. The fallback keeps these
  tag names, IDs, and class names. Do not set `gokuProps` on this layout: that
  makes the shared SDK take its interstitial initialization path.
- **AWS interstitial:** both real scripts loaded with fake `gokuProps`, but the
  invalid context produced no widget. Its `CaptchaScript.renderCaptcha` API
  targets `#captcha-container`; the fallback uses the same AWS modal/form shape.
- **Arkose:** the all-zero key loads the real API and requests `/settings` and
  `/fc/gt2/public_key/<key>`, then calls `onError` without inserting a frame.
  The loaded SDK's frame builder creates an `iframe` with
  `title`/`aria-label="Verification challenge"`,
  `data-e2e="enforcement-frame"`, and
  `client-api.arkoselabs.com/v2/4.6.0/enforcement.<build>.html#<key>`.
  The fallback copies that builder's attributes and real URL. Its frame shape
  comes from the inspected SDK source, not a successfully rendered puzzle.

### GeeTest demo verdicts

`2captcha-geetest-demo` and `2captcha-geetest-v4-demo` wait for a visible
`.geetest_holder`, expect type `geetest` and verdict `solved` (a
`token_applied` the demo then accepts passes too), and need a paid solve: without
`--paid`, an `unsolved` or `unknown` answer is UNVERIFIED. In both deployed pages,
`main form button[type="submit"]` is **Check**. Their client posts to
`/api/v1/captcha-demo/gee-test/verify` (v3) and
`/api/v1/captcha-demo/gee-test-v4/verify` (v4), then renders the returned JSON.
The success patterns are `"success"\s*:\s*true` for v3 and
`"result"\s*:\s*"success"` for v4. Both also display
“Captcha is passed successfully!” in English. These selectors and the client
acceptance paths were inspected in the real browser. A spotting pass does not
claim a solve.

The popup `.geetest_box` can briefly have dimensions before the actual widget
exists; it does not satisfy the demo readiness probe.
