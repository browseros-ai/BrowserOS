// What goto's response says for a page, a missing page, a redirect, a data URL and
// about:blank, and how goto fails for a timeout and a port the browser refuses to load.
const read = async (url, options) =>
  harness.settle(
    page.goto(url, options).then((response) =>
      response === null
        ? null
        : {
            status: response.status(),
            ok: response.ok(),
            statusText: response.statusText(),
            url: response.url(),
            contentType: response.headers()['content-type'] ?? null,
            sameFrame: response.frame() === page.mainFrame(),
            pageUrl: page.url(),
          },
    ),
  )
return {
  page: await read(fixtures.url('child.html')),
  missing: await read(fixtures.url('missing.html')),
  redirect: await read(
    fixtures.url(
      `child.html?redirect=${encodeURIComponent(fixtures.url('child.html?name=Moved'))}`,
    ),
  ),
  data: await read('data:text/html,<p>From data</p>'),
  blank: await read('about:blank'),
  timeout: await read(fixtures.url('slow.html?timeout'), { timeout: 300 }),
  afterTimeout: page.url(),
  unsafePort: await read('http://127.0.0.1:1/'),
}
