const assert = (ok, message) => { if (!ok) throw new Error(message); };
const field = page.getByRef(saved.ref);
assert(await field.evaluate(element => element.id) === 'one', 'evaluate resolves the retained node in a new run');
await field.focus();
await page.keyboard.type('456');
assert(await field.inputValue() === '123456', 'later-run main-frame keys');
const delta = await page.ariaSnapshot({ delta: true });
assert(delta.includes('1 added, 1 removed'), delta);
assert(delta.includes('"123456"'), delta);
for (let i = 0; i < saved.digits.length; i++) {
  await page.getByRef(saved.digits[i]).focus();
  await page.keyboard.type(String(i + 1));
  assert(await page.getByRef(saved.digits[i]).inputValue() === String(i + 1), 'digit keys');
}
await page.getByRef(saved.numberRef).focus();
await page.keyboard.type('286149');
assert(await page.getByRef(saved.numberRef).inputValue() === '286149', 'number keys');
await page.getByRef(saved.frameRef).focus();
await page.keyboard.type('604918');
assert(await page.getByRef(saved.frameRef).inputValue() === '604918', 'cross-origin later-run keys');
assert(await page.getByRef(saved.frameRef).evaluate(element => element.ownerDocument.location.hostname) === 'localhost', 'ref reaches the other origin');
await page.getByRef(saved.buttonRef).evaluate(element => { element.style.pointerEvents = 'auto'; });
await page.getByRef(saved.buttonRef).click();
assert(await page.getByRef(saved.buttonRef).getAttribute('data-clicked') === 'yes', 'minted frame button retains its parent path for click');
const sizeFailure = await field.fill('never', { timeout: 100 }).then(() => '', error => error.message);
assert(/Timeout|timeout|exceeded/.test(sizeFailure), sizeFailure);
assert(await field.inputValue() === '123456', 'fill still waits for visibility and types nothing');
await field.evaluate(element => { element.style.width = '80px'; element.style.height = '20px'; });
const sized = await page.ariaSnapshot();
const engine = await page.getByRole('textbox', { name: 'Verification code', exact: true }).ref();
assert(engine !== saved.ref && /e[1-9]\d*$/.test(engine), `prefer later engine ref ${engine}`);
assert(sized.includes(`[ref=${engine}]`), sized);
assert(await field.inputValue() === '123456', 'minted alias survives the engine ref');
await field.evaluate(element => { const replacement = element.cloneNode(); element.replaceWith(replacement); });
const stale = await field.inputValue().then(() => '', error => error.message);
assert(stale.includes('not on the page any more'), stale);
const replacementRef = await page.getByRole('textbox', { name: 'Verification code', exact: true }).ref();
assert(replacementRef !== saved.ref && replacementRef !== engine, 'same-named replacement gets new identity');
const other = await context.newPage();
try {
  const wrongTab = await other.getByRef(saved.frameRef).inputValue().then(() => '', error => error.message);
  assert(wrongTab.includes('is not from this tab'), wrongTab);
  await other.setContent('<input aria-label="Other">');
  const own = await other.getByRole('textbox', { name: 'Other' }).ref();
  assert(own === 'n1', `first ref on another tab ${own}`);
  assert(await other.getByRef('n1').getAttribute('aria-label') === 'Other', 'bare refs look up the addressed tab');
  await other.getByRef(own).evaluate(element => element.remove());
  const removedWithoutSnapshot = await other.getByRef(own).focus().then(() => '', error => error.message);
  assert(removedWithoutSnapshot.includes('not on the page any more'), removedWithoutSnapshot);
  const third = await context.newPage();
  try {
    await third.setContent('<input aria-label="Third">');
    assert(await third.getByRole('textbox', { name: 'Third' }).ref() === 'n1', 'each tab starts its own n counter');
    assert(await third.getByRef('n1').getAttribute('aria-label') === 'Third', 'two tabs each resolve their own n1');
  } finally { await third.close(); }
} finally { await other.close(); }
await page.goto('data:text/html,replaced document');
const navigated = await page.getByRef(replacementRef).inputValue().then(() => '', error => error.message);
assert(navigated.includes('not on the page any more'), navigated);
return { checked: true, delta };
