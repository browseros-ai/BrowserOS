// Included verbatim by the QuickJS live suite and the hosted/persisted acceptance test.
// The fixture URL is supplied by the owning test; setup uses the SDK's actual edit path.
await page.goto(__FIXTURE_URL__);
globalThis.__neo_upstream_snapshots = true;
const typed = 'TypedCanary-2';
const attribute = 'AttrCanary-1';
const storedText = 'TextCanary-3';
const wrong = 'Wrong expected string without a private marker';
const options = { timeout: 100 };
const input = page.locator('#private-input');
const textarea = page.locator('#private-textarea');
const ancestor = page.locator('#private-ancestor');
const publicInput = page.locator('#public-input');
const publicTextarea = page.locator('#public-textarea');
await input.fill(typed);
await textarea.fill(typed);
await publicInput.fill('PublicTyped');
await publicTextarea.fill('PublicTyped');
await page.locator('#strict-mirror-private').fill(typed);

const passed = [];
const failures = {};
const clearTemplates = [];
const check = (condition, name) => {
  if (!condition) throw new Error('Private failure canary: ' + name);
};
const diagnostic = error => ({
  message: error.message,
  code: error.code,
  actual: error.matcherResult?.actual ?? null,
  log: error.matcherResult?.log ?? null,
  ariaSnapshot: error.matcherResult?.ariaSnapshot ?? null,
});
async function passes(name, call) {
  await call();
  passed.push(name);
}
async function fails(name, call) {
  let failure;
  try { await call(); } catch (error) {
    if (!name.startsWith('toHaveJSProperty.') && name !== 'click.covered' && name !== 'frame.privatePreview' && !name.includes('focus'))
      check(Array.isArray(error.matcherResult?.log) && error.matcherResult.log.length > 0, name + ' forwards structured matcher facts');
    failure = diagnostic(error);
  }
  check(failure !== undefined, name + ' must fail');
  failures[name] = failure;
}
const marker = text => `<${[...text].length} characters>`;

// Snapshot-family assertions compare the same guarded tree that an ordinary read returns.
// Clear templates intentionally supply the canary as Expected; scan their page-derived
// facts independently, because caller-supplied Expected text is allowed to remain clear.
const snapshots = {
  locator: await input.ariaSnapshot({ mode: 'default' }),
  page: await page.ariaSnapshot({ mode: 'default' }),
};
await passes('toMatchAriaSnapshot.locator.copy', () => expect(input).toMatchAriaSnapshot(snapshots.locator, options));
await passes('toMatchAriaSnapshot.page.copy', () => expect(page).toMatchAriaSnapshot(snapshots.page, options));
check(snapshots.locator.includes('•'.repeat(typed.length)), 'locator snapshot has bullets');
for (const [name, subject, template] of [
  ['locator', input, snapshots.locator.replace('•'.repeat(typed.length), typed)],
  ['page', page, snapshots.page.replace('•'.repeat(typed.length), typed)],
]) {
  let failure;
  try { await expect(subject).toMatchAriaSnapshot(template, options); }
  catch (error) {
    check(typeof error.matcherResult?.actual === 'string', name + ' clear template has a received snapshot');
    check(Array.isArray(error.matcherResult?.log) && error.matcherResult.log.length > 0, name + ' clear template has a call log');
    const { message, ...facts } = diagnostic(error);
    check(message.includes(typed), name + ' preserves caller Expected');
    failure = facts;
  }
  check(failure !== undefined, name + ' clear template must fail');
  clearTemplates.push(failure);
}
await fails('toMatchAriaSnapshot.locator', () => expect(input).toMatchAriaSnapshot('- button "Wrong expected string without a private marker"', options));
await fails('toMatchAriaSnapshot.page', () => expect(page).toMatchAriaSnapshot('- button "Wrong expected string without a private marker"', options));
await passes('toHaveAccessibleName.masked', () => expect(page.locator('#name-from-private')).toHaveAccessibleName('Public label ' + '•'.repeat(typed.length), options));
await passes('toHaveAccessibleDescription.masked', () => expect(page.locator('#description-from-private')).toHaveAccessibleDescription('Public label ' + '•'.repeat(typed.length), options));
await passes('toHaveAccessibleErrorMessage.masked', () => expect(page.locator('#error-from-private')).toHaveAccessibleErrorMessage('Wrong: ' + '•'.repeat(typed.length), options));
await fails('toHaveAccessibleErrorMessage', () => expect(page.locator('#error-from-private')).toHaveAccessibleErrorMessage(wrong, options));
check(failures.toHaveAccessibleErrorMessage.actual === 'Wrong: ' + '•'.repeat(typed.length), 'error message received is the guarded value');
await passes('toHaveAccessibleDescription.direct.control', () => expect(page.locator('#description-direct')).toHaveAccessibleDescription('Private textarea', options));
await fails('toHaveAccessibleName', () => expect(page.locator('#name-from-private')).toHaveAccessibleName(wrong, options));
await fails('toHaveAccessibleDescription', () => expect(page.locator('#description-from-private')).toHaveAccessibleDescription(wrong, options));

// Stored textarea text disappears only for snapshot, accessible-name/description and default text comparisons. Its real value
// stays available to a value assertion, and public textarea text remains ordinary text.
await passes('toHaveText.direct.empty', () => expect(textarea).toHaveText('', options));
await passes('toContainText.direct.empty', () => expect(textarea).toContainText('', options));
await passes('toHaveText.ancestor.masked', () => expect(ancestor).toHaveText('Public prefix Public suffix', options));
await passes('toHaveText.array.masked', () => expect(page.locator('.private-array')).toHaveText(['', 'Public tail'], options));
await passes('toContainText.array.masked', () => expect(page.locator('.private-array')).toContainText(['Public tail'], options));
await passes('toHaveValue.textarea.real', () => expect(textarea).toHaveValue(typed, options));
await fails('toHaveText.direct', () => expect(textarea).toHaveText(wrong, options));
await fails('toContainText.direct', () => expect(textarea).toContainText(wrong, options));
await fails('toHaveText.ancestor', () => expect(ancestor).toHaveText(wrong, options));
await fails('toContainText.ancestor', () => expect(ancestor).toContainText(wrong, options));
await fails('toHaveText.array', () => expect(page.locator('.private-array')).toHaveText([wrong, 'Public tail'], options));
await fails('toContainText.array', () => expect(page.locator('.private-array')).toContainText([wrong], options));
// Chromium's native innerText omits textarea child text. These controls preserve that
// behavior without requiring a new innerText interception that native proof did not justify.
await passes('toHaveText.innerText.direct.control', () => expect(textarea).toHaveText('', { ...options, useInnerText: true }));
await passes('toHaveText.innerText.array.control', () => expect(page.locator('.private-array')).toHaveText(['', 'Public tail'], { ...options, useInnerText: true }));
await fails('toHaveText.innerText.ancestor.control', () => expect(ancestor).toHaveText(wrong, { ...options, useInnerText: true }));
await fails('toContainText.innerText.ancestor.control', () => expect(ancestor).toContainText(wrong, { ...options, useInnerText: true }));

// Value-family assertions retain native matches, including patterns, .not and label
// retargeting, while received values and the resolution preview cross the safe boundary.
await passes('toHaveValue.real', () => expect(input).toHaveValue(typed, options));
await passes('toHaveValue.regex', () => expect(input).toHaveValue(/^TypedCanary-2$/, options));
await passes('toHaveValue.not', () => expect(input).not.toHaveValue(wrong, options));
await passes('toHaveValue.label', () => expect(page.locator('#password-label')).toHaveValue(typed, options));
await fails('toHaveValue', () => expect(input).toHaveValue(wrong, options));
await fails('toHaveValue.regex', () => expect(input).toHaveValue(/^Wrong$/, options));
await fails('toHaveValue.not', () => expect(input).not.toHaveValue(/^Typed/, options));
await fails('toHaveValue.label', () => expect(page.locator('#password-label')).toHaveValue(wrong, options));
for (const name of ['value', 'aria-valuetext', 'aria-valuenow']) {
  await passes('toHaveAttribute.' + name + '.real', () => expect(input).toHaveAttribute(name, attribute, options));
  await fails('toHaveAttribute.' + name, () => expect(input).toHaveAttribute(name, wrong, options));
  check(failures['toHaveAttribute.' + name].actual === marker(attribute), name + ' received marker');
}
await passes('toHaveAttribute.caseInsensitive', () => expect(input).toHaveAttribute('VaLuE', attribute, options));
await fails('toHaveAttribute.caseInsensitive', () => expect(input).toHaveAttribute('VaLuE', wrong, options));
await fails('toHaveAttribute.not', () => expect(input).not.toHaveAttribute('value', /^Attr/, options));
await fails('toHaveAttribute.absent', () => expect(input).toHaveAttribute('missing-private', wrong, options));
await fails('toHaveAttribute.publicMetadata', () => expect(input).toHaveAttribute('id', wrong, options));
check(failures['toHaveAttribute.absent'].actual === null, 'absent attribute remains null');
check(failures['toHaveAttribute.publicMetadata'].actual === 'private-input', 'non-value metadata stays readable');
for (const [name, cssOptions, pseudo] of [
  ['own', options, null],
  ['before', { ...options, pseudo: 'before' }, '::before'],
  ['after', { ...options, pseudo: 'after' }, '::after'],
]) {
  const real = await input.evaluate((field, pseudo) => getComputedStyle(field, pseudo).content, pseudo);
  check(real.includes(attribute), 'native CSS ' + name + ' reads attr(value)');
  await passes('toHaveCSS.content.' + name + '.real', () => expect(input).toHaveCSS('content', real, cssOptions));
  await fails('toHaveCSS.content.' + name, () => expect(input).toHaveCSS('content', wrong, cssOptions));
  check(failures['toHaveCSS.content.' + name].actual === marker(real), 'CSS ' + name + ' received marker');
  for (const property of ['CONTENT', 'CoNtEnT']) {
    const key = 'toHaveCSS.' + property + '.' + name;
    await passes(key + '.real', () => expect(input).toHaveCSS(property, real, cssOptions));
    await fails(key, () => expect(input).toHaveCSS(property, wrong, cssOptions));
    check(failures[key].actual === marker(real), key + ' received marker');
  }
}
await fails('toHaveCSS.publicMetadata', () => expect(input).toHaveCSS('color', wrong, options));
check(failures['toHaveCSS.publicMetadata'].actual === 'rgb(20, 40, 60)', 'non-content CSS stays readable');
for (const name of ['toHaveValue', 'toHaveValue.regex', 'toHaveValue.not', 'toHaveValue.label'])
  check(failures[name].actual === marker(typed), name + ' received marker');
await fails('toHaveJSProperty.value', () => expect(input).toHaveJSProperty('value', wrong, options));
await fails('toHaveJSProperty.defaultValue', () => expect(input).toHaveJSProperty('defaultValue', wrong, options));
check(failures['toHaveJSProperty.value'].code === 'unsupported', 'value property remains refused');
check(failures['toHaveJSProperty.defaultValue'].code === 'unsupported', 'defaultValue property remains refused');

// These paths print previews independently of a value's Received line. Missing-element
// assertions also build a whole-body fallback tree; strict errors build an `aka` alias.
await fails('toBeEmpty', () => expect(input).toBeEmpty({ timeout: 1000 }));
await fails('toBeHidden', () => expect(input).toBeHidden({ timeout: 1000 }));
await fails('toBeEmpty.textarea', () => expect(textarea).toBeEmpty(options));
await fails('click.covered', () => page.locator('#covered-input').click({ timeout: 1000 }));
await fails('missing.fallback', () => expect(page.locator('#missing-private-field')).toHaveText(wrong, options));
await fails('strict.mirrored.fields', () => expect(page.locator('.strict-mirror-fields')).toHaveValue(wrong, options));
check(failures['strict.mirrored.fields'].message.includes('value="' + marker(typed) + '"'), 'strict violation projects the mirrored typed attribute');
await fails('strict.fields', () => expect(page.locator('.strict-fields')).toHaveValue(wrong, options));
await fails('strict.alias', () => expect(page.locator('.strict-names')).toHaveText(wrong, options));
await fails('frame.privatePreview', () => page.locator('#private-input').contentFrame().locator('body').textContent(options));
await fails('fill.focus', () => page.locator('#focus-victim').fill('Public attempt', { timeout: 1000 }));
await fails('type.focus', () => page.locator('#focus-victim').pressSequentially('Public attempt', { timeout: 1000 }));
await fails('public.fill.focus', () => page.locator('#public-focus-victim').fill('Public attempt', { timeout: 1000 }));
for (const name of ['fill.focus', 'type.focus']) {
  check(failures[name].code === 'not_focused', name + ' reports focus loss after its microtask');
  check(failures[name].message.includes('value="' + marker(attribute) + '"'), name + ' projects the private focus holder');
}
check(failures['public.fill.focus'].message.includes('value="PublicFocus"'), 'an ordinary focus holder keeps its attribute');
for (const name of ['toBeEmpty', 'toBeHidden', 'click.covered', 'strict.fields']) {
  for (const attributeName of ['value', 'aria-valuetext', 'aria-valuenow'])
    check(failures[name].message.includes(attributeName + '="' + marker(attribute) + '"'), name + ' masks ' + attributeName);
}
check(failures['strict.alias'].message.includes(' aka '), 'strict alias path was exercised');
check(failures['missing.fallback'].ariaSnapshot?.includes('•'), 'missing locator has a guarded body fallback');
check(failures['click.covered'].message.includes('intercepts pointer events'), 'covered click reports hit testing');
for (const name of ['toHaveValue', 'toHaveText.direct', 'toHaveText.ancestor', 'toHaveAccessibleName', 'toHaveAccessibleDescription', 'toBeHidden'])
  check(typeof failures[name].ariaSnapshot === 'string' && failures[name].ariaSnapshot.length > 0, name + ' forwards a failure snapshot');

// Public twins establish that the guard did not globally suppress received values, text,
// attributes or names. Intentional private reads remain clear after all failure cleanups.
await passes('public.toHaveAccessibleErrorMessage', () => expect(page.locator('#error-from-public')).toHaveAccessibleErrorMessage('Wrong: PublicTyped', options));
await fails('public.toHaveAccessibleErrorMessage', () => expect(page.locator('#error-from-public')).toHaveAccessibleErrorMessage(wrong, options));
await passes('public.toHaveText', () => expect(publicTextarea).toHaveText('PublicText', options));
await passes('public.toContainText', () => expect(page.locator('#public-ancestor')).toContainText('PublicText', options));
await passes('public.toHaveText.array', () => expect(page.locator('.public-array')).toHaveText(['PublicText', 'Public tail'], options));
await passes('public.toContainText.array', () => expect(page.locator('.public-array')).toContainText(['PublicText'], options));
await passes('public.toHaveAccessibleName', () => expect(page.locator('#name-from-public')).toHaveAccessibleName('Public label PublicTyped', options));
await passes('public.toHaveAccessibleDescription', () => expect(page.locator('#description-from-public')).toHaveAccessibleDescription('Public label PublicTyped', options));
await fails('public.toHaveValue', () => expect(publicInput).toHaveValue(wrong, options));
await fails('public.toHaveText', () => expect(publicTextarea).toHaveText(wrong, options));
await fails('public.toHaveAttribute', () => expect(publicInput).toHaveAttribute('value', wrong, options));
await fails('public.toHaveAccessibleName', () => expect(page.locator('#name-from-public')).toHaveAccessibleName(wrong, options));
await fails('public.toHaveAccessibleDescription', () => expect(page.locator('#description-from-public')).toHaveAccessibleDescription(wrong, options));
await fails('public.toHaveCSS', () => expect(publicInput).toHaveCSS('content', wrong, options));
for (const [name, actual] of [
  ['public.toHaveAccessibleErrorMessage', 'Wrong: PublicTyped'], ['public.toHaveValue', 'PublicTyped'], ['public.toHaveText', 'PublicText'],
  ['public.toHaveAttribute', 'PublicAttr'], ['public.toHaveAccessibleName', 'Public label PublicTyped'],
  ['public.toHaveAccessibleDescription', 'Public label PublicTyped'], ['public.toHaveCSS', '"PublicAttr"'],
]) check(failures[name].actual === actual, name + ' preserves public received value');
const reads = {
  inputValue: await input.inputValue(),
  attribute: await input.getAttribute('value'),
  textareaValue: await textarea.inputValue(),
  textareaText: await textarea.evaluate(field => field.textContent),
  evaluated: await input.evaluate(field => field.value),
  content: await page.content(),
};
check(reads.inputValue === typed && reads.evaluated === typed, 'explicit value reads stay clear');
check(reads.attribute === attribute, 'explicit attribute stays clear');
check(reads.textareaValue === typed && reads.textareaText === storedText, 'textarea live and stored text stay clear');
check(reads.content.includes(attribute) && reads.content.includes(storedText), 'explicit markup stays clear');
for (const [name, failure] of Object.entries({ ...failures, clearTemplates, snapshots })) {
  for (const canary of [typed, attribute, storedText])
    check(!JSON.stringify(failure).includes(canary), name + ' diagnostic contains no private canary');
}
return { failures, clearTemplates, snapshots, reads, passed };
