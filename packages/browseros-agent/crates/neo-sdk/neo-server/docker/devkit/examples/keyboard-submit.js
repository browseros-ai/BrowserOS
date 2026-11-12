// Type with the keyboard and submit with Enter, then wait for the page to show the result.
const html = `<title>Search</title>
<form onsubmit="event.preventDefault(); document.querySelector('#out').textContent = 'Results for ' + this.q.value">
  <input name="q" aria-label="Search">
</form>
<p id="out"></p>`;
await page.goto('data:text/html,' + encodeURIComponent(html));
await page.getByLabel('Search').click();
await page.keyboard.type('claw sdk');
await page.keyboard.press('Enter');
await expect(page.locator('#out')).toHaveText('Results for claw sdk');
return { shown: await page.locator('#out').textContent() };
