# CAPTCHA fixtures

From `packages/browseros-agent`:

```sh
bun run captcha:fixtures
bun run captcha:fixtures --host 0.0.0.0 --public-host host.docker.internal --port 18080
bun test contracts/captcha/fixtures
```

The CLI prints fixture and demo URLs. Default bind/public host: `127.0.0.1`; default port: random in 10101–20202. `--host` selects the listener; `--public-host` selects advertised URLs. Ctrl-C stops the server.

Each standalone page loads its vendor's real script. It mirrors widget state into `window.__captchaFixture` and `document.documentElement.dataset.captchaFixture`. Success records token length, while errors and expiry clear it. Rendering tracks the visible iframe, or Turnstile's visible host when the iframe has a closed shadow root. Submissions use real vendor siteverify endpoints and return `ACCEPTED` or `REJECTED: <codes>` with `data-captcha-verdict`. `verifyUrls` substitutes local endpoints in unit tests.

## Pages and public test keys

| Page | Proves | Sitekey |
|---|---|---|
| `recaptcha-v2.html` | Checkbox completion and server acceptance | `6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI` |
| `turnstile-pass.html` | Automatic completion | `1x00000000000000000000AA` |
| `turnstile-block.html` | Failure without a token | `2x00000000000000000000AB` |
| `turnstile-interactive.html` | Completion after checkbox interaction | `3x00000000000000000000FF` |
| `no-captcha.html` | Ordinary form; always accepted | None |
| `hcaptcha.html` | hCaptcha detection and unsupported-family handling in solver v1 | `10000000-ffff-ffff-ffff-000000000001` |

Public test secrets, verified against vendor documentation:

- [Google FAQ](https://developers.google.com/recaptcha/docs/faq#id-like-to-run-automated-tests-with-recaptcha.-what-should-i-do): `6LeIxAcTAAAAAGG-vFI1TnRWxMZNFuojJ4WifJWe`. The original brief's sitekey was invalid; the catalog uses Google's documented key, approved in the shared contract.
- [Cloudflare testing](https://developers.cloudflare.com/turnstile/troubleshooting/testing/): pass/interactive `1x0000000000000000000000000000000AA`; block `2x0000000000000000000000000000000AA`.
- [hCaptcha integration testing](https://docs.hcaptcha.com/#integration-testing-test-keys): `0x0000000000000000000000000000000000000000`.

## Browser checks

Checked with a temporary BrowserOS neo profile over CDP, without using the owner's browser. reCAPTCHA, Turnstile pass/interactive, hCaptcha and the ordinary form returned `ACCEPTED`. Turnstile block rendered, reported `600010`, produced no token, and returned `REJECTED: missing-input-response`. The hCaptcha test checkbox was completed manually; solver v1 still does not support it.

[hCaptcha recommends a named local host](https://docs.hcaptcha.com/#local-development). This build rendered and accepted the test token on both `127.0.0.1` and `captcha.test`. For environments enforcing its localhost/IP restrictions, serve with `--public-host captcha.test` and launch your test Chromium with `--host-resolver-rules="MAP captcha.test 127.0.0.1"`.

## Demo checks

Fetched with curl and opened in an isolated browser:

- [Google](https://www.google.com/recaptcha/api2/demo): real key; `#recaptcha-demo-submit`; acceptance text `Verification Success`.
- [Cloudflare](https://demo.turnstile.workers.dev/): public pass key, visible widget host, form posts to `/handler`; submitting its dummy token returned JSON with `"success":true`.
- [hCaptcha](https://accounts.hcaptcha.com/demo): real key, rendered checkbox; no v1 acceptance check.
- [2Captcha reCAPTCHA v2](https://2captcha.com/demo/recaptcha-v2): live widget `#g-recaptcha`, real sitekey `6LfD3PIbAAAAAJs_eEHvoOl75_83eXSqpPSRFJ_u`; may need a paid solve.
- [2Captcha Turnstile](https://2captcha.com/demo/cloudflare-turnstile): live widget `#cf-turnstile`, public interactive **test key** `3x00000000000000000000FF`; no paid solve required. Readiness checks the visible host because its iframe is inside a closed shadow root.

Both 2Captcha demos were inspected in BrowserOS neo. Their `main form button[type="submit"]` checks the token with `/api/v1/captcha-demo/<slug>/verify`; the client renders the server's JSON with `"success": true` only after acceptance. The Turnstile test checkbox was completed and submitted: its real response contained `"success": true` and `"result_with_testing_key": true`. The reCAPTCHA acceptance path was read from the deployed client; no paid demo solve was attempted. Inspecting these public demos needs no provider key.

CapSolver's own-domain demo search found guides pointing to external demos, not a hosted demo. `demo.capsolver.com` did not resolve; no speculative entry was added. No third-party production site is included.

Provider readiness is checked separately by `bun run captcha:provider-check`.
Both providers passed the live API check; paid solving on these demo pages remains **unverified**. These fixtures never read provider API keys.
