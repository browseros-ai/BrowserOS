// A local file. Refused: Neo opens only http, https, data and about URLs, since a
// file in the container could show Neo's own secrets.
await page.goto('file:///etc/passwd');
return { content: await page.content() };
