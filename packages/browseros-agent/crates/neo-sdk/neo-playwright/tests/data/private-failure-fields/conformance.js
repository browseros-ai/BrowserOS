await page.goto(__FIXTURE_URL__);
// The test host set this before the prelude started. A script cannot change that capture.
globalThis.__neo_upstream_snapshots = false;
const input = page.locator('#private-input');
const textarea = page.locator('#private-textarea');
await input.fill('TypedCanary-2');
await textarea.fill('TypedCanary-2');
const wrong = 'Wrong expected string without a private marker';
const options = { timeout: 100 };
const failures = {};
async function fails(name, call) {
  try { await call(); }
  catch (error) {
    if (!Array.isArray(error.matcherResult?.log) || error.matcherResult.log.length === 0)
      throw new Error(name + ' lost the native structured matcher facts');
    failures[name] = {
      message: error.message,
      actual: error.matcherResult?.actual ?? null,
      log: error.matcherResult?.log ?? null,
      ariaSnapshot: error.matcherResult?.ariaSnapshot ?? null,
    };
    return;
  }
  throw new Error(name + ' unexpectedly passed under conformance');
}
const snapshots = {
  locator: await input.ariaSnapshot({ mode: 'default' }),
  page: await page.ariaSnapshot({ mode: 'default' }),
};
await expect(input).toMatchAriaSnapshot(snapshots.locator, options);
await expect(page).toMatchAriaSnapshot(snapshots.page, options);
await fails('toHaveValue', () => expect(input).toHaveValue(wrong, options));
await fails('toHaveAttribute.value', () => expect(input).toHaveAttribute('value', wrong, options));
await fails('toHaveAttribute.aria-valuetext', () => expect(input).toHaveAttribute('aria-valuetext', wrong, options));
await fails('toHaveAttribute.aria-valuenow', () => expect(input).toHaveAttribute('aria-valuenow', wrong, options));
await fails('toHaveText.direct', () => expect(textarea).toHaveText(wrong, options));
await fails('toHaveText.ancestor', () => expect(page.locator('#private-ancestor')).toHaveText(wrong, options));
await fails('toContainText.direct', () => expect(textarea).toContainText(wrong, options));
await fails('toHaveText.array', () => expect(page.locator('.private-array')).toHaveText([wrong, 'Public tail'], options));
await fails('toContainText.array', () => expect(page.locator('.private-array')).toContainText([wrong], options));
await fails('toHaveAccessibleName', () => expect(page.locator('#name-from-private')).toHaveAccessibleName(wrong, options));
await fails('toHaveAccessibleErrorMessage', () => expect(page.locator('#error-from-private')).toHaveAccessibleErrorMessage(wrong, options));
await fails('toHaveAccessibleDescription', () => expect(page.locator('#description-from-private')).toHaveAccessibleDescription(wrong, options));
await fails('toHaveCSS.content.own', () => expect(input).toHaveCSS('content', wrong, options));
await fails('toHaveCSS.content.before', () => expect(input).toHaveCSS('content', wrong, { ...options, pseudo: 'before' }));
await fails('toHaveCSS.content.after', () => expect(input).toHaveCSS('content', wrong, { ...options, pseudo: 'after' }));
await fails('toMatchAriaSnapshot.locator', () => expect(input).toMatchAriaSnapshot('- button "Wrong expected string without a private marker"', options));
await fails('toMatchAriaSnapshot.page', () => expect(page).toMatchAriaSnapshot('- button "Wrong expected string without a private marker"', options));
await fails('toBeEmpty', () => expect(input).toBeEmpty(options));
await fails('toBeHidden', () => expect(input).toBeHidden(options));
await fails('missing.fallback', () => expect(page.locator('#missing-private-field')).toHaveText(wrong, options));
await fails('strict.fields', () => expect(page.locator('.strict-fields')).toHaveValue(wrong, options));
await fails('strict.alias', () => expect(page.locator('.strict-names')).toHaveText(wrong, options));
return { failures, snapshots };
