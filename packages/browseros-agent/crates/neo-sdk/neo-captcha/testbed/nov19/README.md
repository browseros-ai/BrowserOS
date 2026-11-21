# Nov 19 CAPTCHA test bench

Cases where Neo's CAPTCHA solver said a page was solved or clear while it was still blocked, from the
Nov 19 eval runs. Data only: nothing runs these yet, and today's solver is expected to get several wrong.

## Pages (`pages/`)

Self-contained fixture pages that reproduce each failure. `expect` is the answer a correct solver gives.
The Turnstile pages use Cloudflare's always-fail test sitekeys (`2x00000000000000000000AB`, `...BB`); a
server that serves them must verify those with the always-fail secret, so no token passes.

| Page | What it models | Vendor | Expect |
|---|---|---|---|
| `policy-wall-ai-agent.html` | Access restricted for AI agents | none | policy_wall |
| `policy-wall-amazon-nov19.html` | Amazon access restriction recorded on November 19 | none | policy_wall |
| `policy-wall-terms-footer.html` | Ordinary storefront with terms in the footer | none | already_clear |
| `turnstile-frame-never-loads.html` | Turnstile beside a frame that never loads | turnstile | solved, already_clear |
| `turnstile-hidden-verifying.html` | Turnstile checking out of sight | turnstile | unsolved, unknown |
| `turnstile-stale-token.html` | Turnstile failed beside a stale token | turnstile | unsolved, unknown |
| `verify-alert-only.html` | Only an alert says the check failed | turnstile | unsolved, unknown |
| `verify-second-challenge.html` | A second Turnstile check opens after the first passes | turnstile | unsolved, unknown |

## Recorded cases (`cases/`)

One JSON file per case. `truth` is what the page really showed; `claim` is what Neo answered on the run;
`source` names the trace it came from. `look` is a recorded summary of the page and `expect` is the
output an earlier design (#3772) expected; a new design can ignore both and keep `truth`.

| Case | Truth | Neo claimed |
|---|---|---|
| `01-ny-failure-dialog-already-passed` | blocked | clear: the widget looked passed (turnstile) |
| `02-nc-failure-dialog-after-token` | blocked | clear: Neo put a token in the page (turnstile) |
| `03-fl-puzzle-over-search-after-token` | blocked | clear: Neo put a token in the page (recaptcha_v2) |
| `04-fl-expired-checkbox-after-token` | blocked | clear: Neo put a token in the page (recaptcha_v2) |
| `05-amazon-wall-after-a-flow` | policy | clear: the widget looked passed (turnstile) |
| `06-amazon-wall-reworded-for-jev` | policy | clear: found no CAPTCHA |
| `07-yelp-datadome-not-blocking` | blocked | clear: found no CAPTCHA |
| `08-seats-cloudflare-wall` | blocked | clear: the Cloudflare wall looked gone (cloudflare_challenge) |
| `09-mm-hidden-check-still-running` | checking | clear: Neo put a token in the page (turnstile) |
| `10-mm-please-wait-unreadable-widget` | checking | clear: Neo put a token in the page (turnstile) |
| `11-yelp-device-check-running` | checking | clear: found no CAPTCHA |
| `12-mm-failure-alert-without-dialog` | blocked | clear: found no CAPTCHA |
| `13-ny-results-clear` | clear | clear: found no CAPTCHA |
| `14-mm-results-clear` | clear | clear: found no CAPTCHA |
| `15-mm-results-hidden-widget-for-jev` | clear | clear: the widget looked passed (turnstile) |
| `16-fl-results-clear` | clear | clear: the widget looked passed (recaptcha_v2) |
| `17-yelp-reservation-clear` | clear | clear: found no CAPTCHA |
| `18-ohio-results-clear` | clear | clear: found no CAPTCHA |
| `19-airfrance-home-with-badge-clear` | clear | clear: found no CAPTCHA |
| `20-airfrance-cookie-dialog` | clear | clear: found no CAPTCHA |
| `21-mm-privacy-center-over-results` | clear | clear: found no CAPTCHA |
| `22-amazon-choose-location` | clear | clear: found no CAPTCHA |
| `23-seats-calendar-announcement` | clear | clear: found no CAPTCHA |
| `24-dtf-newsletter-modal` | clear | clear: found no CAPTCHA |
| `25-fl-bus-puzzle-after-a-pass-claim` | blocked | clear: the widget looked passed (recaptcha_v2) |
| `26-nc-failure-dialog-frame-unseen` | blocked | clear: Neo put a token in the page (turnstile) |
| `27-synthetic-links-in-page-text` | clear | clear: found no CAPTCHA |
