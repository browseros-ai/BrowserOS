// Load a page in this run's `page` and read it back. With --task, later runs of
// the same task get this tab as their `page`.
await page.goto('https://example.com');
return { title: await page.title(), url: page.url() };
