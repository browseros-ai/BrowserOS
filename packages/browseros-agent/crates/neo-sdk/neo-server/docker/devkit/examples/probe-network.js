// Waiting for a network response. Not available: Neo reports no request or response events and
// cannot intercept requests (page.waitForRequest, page.on('request'), page.route are refused the
// same way). The Response that page.goto returns does work: status(), ok(), url().
const waiting = page.waitForResponse(() => true, { timeout: 3000 });
await page.goto('https://example.com');
return (await waiting).status();
