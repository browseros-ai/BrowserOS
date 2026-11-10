// Snapshots of sensitive.html, whose password, card and one-time-code fields hold values and
// whose text echoes the code: what each mode prints for them, and their textContent.
await page.goto(fixtures.url('sensitive.html'))
return {
  snapshot: await harness.settle(page.ariaSnapshot()),
  ai: await harness.settle(page.ariaSnapshot({ mode: 'ai' })),
  echo: await harness.settle(page.locator('#echo').textContent()),
}
