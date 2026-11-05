// Script in the page's main world. Held for privacy (script / method_unavailable):
// read through locators, page.content() or page.title() instead.
await page.goto('data:text/html,' + encodeURIComponent('<title>Evaluate</title>'));
return await page.evaluate(() => document.title);
