// Read the prior run's baseline after changing only a private value length. Deliberate
// clear-value reads remain separate from the snapshot canary scan.
try {
  await page.evaluate(() => { window.adoption.fields[0].value += '!'; });
  const delta = await page.ariaSnapshot({ delta: true, text: false });
  const child = page.locator('#frame-host').locator('#adoption-child').contentFrame().locator('body');
  const destination = __DESTINATION__ === 'child' ? child : page.locator('body');
  return { delta, reads: [{
    direct: await Promise.all(['private-password', 'private-textarea'].map(id => destination.locator('#' + id).inputValue())),
    main: await page.evaluate(() => window.adoption.read()),
  }] };
} finally { await page.close(); }
