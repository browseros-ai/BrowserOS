// Set a checkbox. The SDK runs setChecked(true) as check() and setChecked(false) as uncheck().
await page.goto('data:text/html,' + encodeURIComponent('<title>Check</title><input id="c" type="checkbox">'));
await page.locator('#c').setChecked(true);
return await page.locator('#c').isChecked();
