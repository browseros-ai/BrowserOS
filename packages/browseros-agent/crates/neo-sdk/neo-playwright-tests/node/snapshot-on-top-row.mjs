/** One agent script drives each fixture in Node and in the Rust note-recording host. */
const assert = {
  ok(value, message) {
    if (!value) throw new Error(message)
  },
  equal(actual, expected, message) {
    if (actual !== expected)
      throw new Error(
        `${message}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`,
      )
  },
  notEqual(actual, expected, message) {
    if (actual === expected) throw new Error(message)
  },
}

function checkNotes(text, notes, withBody = false) {
  const lines = text.split('\n').filter((line) => line.startsWith('# on top: '))
  assert.equal(lines.length, notes.length, text)
  for (const [index, note] of notes.entries()) {
    const prefix = `# on top: ${note.role}${note.name ? ` ${JSON.stringify(note.name)}` : ''} [ref=`
    assert.ok(lines[index].startsWith(prefix), `${prefix}\n${text}`)
    assert.ok(
      lines[index].endsWith(
        note.extent === 'whole'
          ? ' over the whole view'
          : ' over part of the view',
      ),
      text,
    )
    for (const flag of note.flags)
      assert.ok(lines[index].includes(`, ${flag}`), text)
    for (const flag of ['blank', 'loading'])
      if (!note.flags.includes(flag))
        assert.ok(!lines[index].includes(`, ${flag}`), text)
    if (note.says)
      assert.ok(
        lines[index].includes(`, says ${JSON.stringify(note.says)}`),
        text,
      )
    const following =
      text.split('\n')[text.split('\n').indexOf(lines[index]) + 1] ?? ''
    if (note.controls.length) {
      assert.ok(following.startsWith('#   its controls: '), text)
      for (const name of note.controls.slice(0, 4)) {
        if (name) assert.ok(following.includes(JSON.stringify(name)), text)
        else assert.ok(following.includes('button [ref='), text)
      }
      if (note.controlText && withBody) {
        const ref = /button \[ref=([^\]]+)\]/.exec(following)?.[1]
        assert.ok(ref, text)
        const line = `- button [ref=${ref}]: ${note.controlText}`
        const at = text.indexOf(line)
        assert.ok(
          at >= 0 && at < text.indexOf('- button "Open information"'),
          text,
        )
      }
    } else assert.ok(!following.startsWith('#   its controls: '), text)
    for (const name of note.forbidControls ?? [])
      assert.ok(!following.includes(JSON.stringify(name)), text)
  }
}

export async function driveSnapshotRow(page, row, url) {
  const outputs = []
  const expectedCounts = []
  await page.setViewportSize(row.viewport)
  await page.goto(url)
  await page.waitForFunction(() => window.fixtureReady === true)
  // Main lists a closed root after the engine knows it. Prime through a locator,
  // which opens the root without a page survey, before comparing upstream reads.
  if (row.primeClosedRoots) {
    const primed = await page.locator('body').ariaSnapshot({ mode: 'full' })
    assert.ok(
      primed.includes('Close private overlay'),
      'locator primer must open the fixture root',
    )
  }
  const original = await page.ariaSnapshot({ mode: 'default' })
  const snap = async (count, options = {}) => {
    const text = await page.ariaSnapshot(options)
    expectedCounts.push(count)
    outputs.push(text)
    return text
  }
  const initial = await snap(row.expectedControls)
  checkNotes(initial, row.expectedNotes, true)
  if (row.firstText) {
    assert.ok(initial.indexOf(row.firstText) >= 0, initial)
    assert.ok(
      initial.indexOf(row.firstText) < initial.indexOf('Outside box'),
      initial,
    )
    assert.equal(initial.split(row.firstText).length - 1, 1, initial)
  }
  if (row.longPortal) {
    assert.ok(
      initial.startsWith('# on top: dialog "Log in or sign up"'),
      initial,
    )
    assert.ok(initial.slice(0, 400).includes('button "Close"'), initial)
    assert.ok(
      initial.indexOf('- dialog "Log in or sign up"') <
        initial.indexOf('Reserve Tickets'),
      initial,
    )
  }
  for (let i = 1; i < (row.bodyOrder?.length ?? 0); i++) {
    const previous = initial.indexOf(row.bodyOrder[i - 1])
    const current = initial.indexOf(row.bodyOrder[i])
    assert.ok(previous >= 0 && previous < current, initial)
  }
  const full = await snap(row.expectedControls, { mode: 'full' })
  checkNotes(full, row.expectedNotes)
  const json = await page.ariaSnapshotJSON({ mode: 'ai' })
  assert.ok(
    !JSON.stringify(json).includes('"onTop":'),
    'JSON mode must not carry Neo content marks',
  )
  if (row.noRefDialog) {
    const pending = [...json]
    let dialog
    while (pending.length) {
      const node = pending.pop()
      if (!node || typeof node !== 'object') continue
      if (node.role === 'dialog' && node.name === row.noRefDialog) dialog = node
      for (const child of node.children ?? [])
        if (typeof child === 'object') pending.push(child)
    }
    assert.ok(dialog, 'the engine must list the no-box dialog')
    assert.equal(
      dialog.ref,
      undefined,
      'the fixture must exercise a dialog with no ref',
    )
  }
  assert.equal(
    await page.ariaSnapshot({ mode: 'default' }),
    original,
    'upstream mode must be byte-identical',
  )
  for (const maxTokens of [1, 8, 80])
    checkNotes(
      await snap(row.expectedControls, { maxTokens }),
      row.expectedNotes,
    )
  for (const phase of row.phases ?? []) {
    if (phase.after.startsWith('scroll')) {
      const y = Number(/y=(\d+)/.exec(phase.after)?.[1] ?? 0)
      const before = await page.evaluate(() => window.scrollY)
      await page.evaluate((y) => window.scrollTo(0, y), y)
      const after = await page.evaluate(() => window.scrollY)
      assert.notEqual(after, before, 'the phase must actually scroll')
      assert.ok(
        Math.abs(after - y) < 1,
        'the phase reaches its named scroll position',
      )
    } else if (phase.after.startsWith('press ')) {
      const name = phase.after.slice('press '.length)
      const control = page.getByRole('button', { name, exact: true })
      assert.ok(await control.isVisible(), `the phase's ${name} is shown`)
      await control.click()
      assert.equal(
        await control.isVisible(),
        false,
        `the phase's ${name} must dismiss its box`,
      )
    } else {
      assert.equal(
        phase.after,
        'next snapshot while toast stands',
        'unknown fixture action',
      )
    }
    checkNotes(await snap(phase.expectedControls), phase.notes)
  }
  return { outputs, expectedCounts }
}
