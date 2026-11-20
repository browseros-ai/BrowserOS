# Every image a person tests: keys in, then the CAPTCHA test

## Keys always in

Any image a person will test, or a customer will pull, is built keyed:

- Local or Fly builds: `build.sh --capsolver-key-file FILE --twocaptcha-key-file FILE`.
- Published images: the release workflow `neo-sdk-image.yml` with `publish=true`, which takes
  both keys from the `neo-captcha` environment.

Check it: `/usr/share/neo-sdk/provenance.json` → `captchaKeys` is `["capsolver","2captcha"]`. A
keyless image fails the test below.

Never write key values into the repo, docs, logs or PRs.

## The CAPTCHA test, after every new image build

Run it with the owner watching the live view. Paste this to the agent verbatim:

```
I'm testing CAPTCHA handling. Open a browser and visit the links below one after another, in this exact order, so I can watch and solve each CAPTCHA with the tool. Give me a browser link so I can view the session live.
After each CAPTCHA is solved, wait 15 seconds before moving to the next one.
    1    reCAPTCHA v2 checkbox: https://www.google.com/recaptcha/api2/demo
    2    reCAPTCHA v2 invisible: https://www.google.com/recaptcha/api2/demo?invisible=true
    3    reCAPTCHA v2 Enterprise: https://2captcha.com/demo/recaptcha-v2-enterprise
    4    Cloudflare Turnstile: https://demo.turnstile.workers.dev/
    5    reCAPTCHA v3: https://recaptcha-demo.appspot.com/recaptcha-v3-request-scores.php
    6    Cloudflare full-page challenge: https://www.scrapingcourse.com/cloudflare-challenge
```

A site passes only when Neo's solver solved it through a provider; name the provider, from Neo's
run output or logs. A site anyone clicked by hand fails.

Record pass or fail per site, and the provider, next to the image tag before calling the image
good.
