// Fill a form, click its button, and assert what the page shows: Playwright actions and a
// web-first assertion. Each action is one operation in the trace, with its delivery.
const html = `<title>Order</title>
<form onsubmit="event.preventDefault(); document.querySelector('#out').textContent =
  'Thanks, ' + this.elements.name.value + ' (' + this.elements.email.value + ')'">
  <label>Name <input name="name"></label>
  <label>Email <input name="email" type="email"></label>
  <button>Send</button>
</form>
<p id="out"></p>`;
await page.goto('data:text/html,' + encodeURIComponent(html));
await page.getByLabel('Name').fill('Ada Lovelace');
await page.getByLabel('Email').fill('ada@example.com');
await page.getByRole('button', { name: 'Send' }).click();
await expect(page.locator('#out')).toHaveText('Thanks, Ada Lovelace (ada@example.com)');
return { shown: await page.locator('#out').textContent() };
