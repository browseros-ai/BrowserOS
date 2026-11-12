// Clear a field. The SDK runs locator.clear() as fill(''), so the trace shows a locator.fill.
await page.goto('data:text/html,' + encodeURIComponent('<title>Clear</title><input id="t" value="abc">'));
await page.locator('#t').clear();
return await page.locator('#t').inputValue();
