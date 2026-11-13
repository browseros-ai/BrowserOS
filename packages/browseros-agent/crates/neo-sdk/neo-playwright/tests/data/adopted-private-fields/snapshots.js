// Both suites warm the source worlds before moving the controls. Native observers split
// this shared flow at adoption; the server suite persists the complete flow as one run.
await p.goto(__FIXTURE_URL__);
const initial = [await p.ariaSnapshot({ mode: 'ai' })];
const child = p.locator('#frame-host').locator('#adoption-child').contentFrame().locator('body');
await child.locator('#ready').waitFor();
initial.push(await child.ariaSnapshot({ mode: 'ai' }));
const source = __DESTINATION__ === 'child' ? p.locator('body') : child;
const name = __CASE_NAME__;
const values = [
  `AdoptedPrivate-${name}-password-value🔒`, `AdoptedPrivate-${name}-textarea-value🔒`,
  `AdoptedPublic-${name}-input-value`, `AdoptedPublic-${name}-textarea-value`,
];
for (const [index, id] of ['private-password', 'private-textarea', 'public-input', 'public-textarea'].entries())
  await source.locator('#' + id).fill(values[index]);
const facts = await p.evaluate(() => window.adoption.adopt());
const destination = __DESTINATION__ === 'child' ? child : p.locator('body');
const snapshots = {};
const reads = [];
for (const [scope, target] of [
  ['page', p], ['destination', destination],
  ['passwordLabel', destination.locator('#password-label')],
  ['textareaLabel', destination.locator('#textarea-label')],
  ['passwordOwner', destination.locator('#password-owner')],
  ['textareaOwner', destination.locator('#textarea-owner')],
]) {
  const texts = {};
  for (const [format, options] of [
    ['automatic', {}], ['ai', { mode: 'ai' }], ['text', { text: true }],
    ['noText', { text: false }], ['full', { mode: 'full' }], ['default', { mode: 'default' }],
  ]) texts[format] = await target.ariaSnapshot(options);
  const trees = {};
  for (const mode of ['ai', 'default']) trees[mode] = await target.ariaSnapshotJSON({ mode });
  snapshots[scope] = { texts, trees };
  reads.push({
    direct: await Promise.all(['private-password', 'private-textarea'].map(id => destination.locator('#' + id).inputValue())),
    main: await p.evaluate(() => window.adoption.read()),
  });
}
await destination.locator('#password-label').focus();
const baseline = await p.ariaSnapshot({ text: false });
return { page: p, initial, facts, snapshots, reads, baseline };
