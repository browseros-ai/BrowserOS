// Run a script in the page's main world and return its result.
await page.goto('data:text/html,' + encodeURIComponent('<title>Evaluate</title>'));
return await page.evaluate(() => document.title);
