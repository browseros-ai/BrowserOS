// setInputFiles on files.html with paths, payloads, `[]` and a directory, through every form,
// and every error of upstream's table that a co-located client can reach, with the events
// each path fires and the files the page then reads. Both runners work in the conformance
// directory, so `files/…` paths are relative to it, as a script's are to Neo's.
await page.goto(fixtures.url('files.html'))
const files = (id) => harness.pageEval(page, `window.__files(${JSON.stringify(id)})`)
const events = () => harness.pageEval(page, 'window.__takeEvents()')
const bytes = (text) => new Uint8Array([...text].map((c) => c.charCodeAt(0)))
// A payload's File is stamped when it is built, so only whether that was just now compares.
const stamped = async (id) => {
  const now = await harness.pageEval(page, 'Date.now()')
  return (await files(id)).map(({ lastModified, ...file }) => ({
    ...file,
    recent: Math.abs(lastModified - now) < 60_000,
  }))
}
const attempt = async (work, id) => ({
  outcome: await harness.settle(work),
  files: id ? await files(id) : undefined,
  events: await events(),
})
const out = {}

// Paths, set natively: trusted events, and the file's own name, type, bytes and mtime. Setting
// the file an input already holds changes nothing, so Chromium fires `cancel` instead.
out.path = await attempt(page.setInputFiles('#single', 'files/one.txt'), 'single')
out.spaced = await attempt(page.locator('#single').setInputFiles('files/space name.txt'), 'single')
out.again = await attempt(page.setInputFiles('#single', './files/../files/two.txt'), 'single')
out.multiple = await attempt(
  page.mainFrame().setInputFiles('#multi', ['files/one.txt', 'files/photo.png']),
  'multi',
)
out.wrapped = await attempt(page.setInputFiles('#wrap', 'files/one.txt'), 'wrapped')
out.labelFor = await attempt(page.setInputFiles('#for-label', 'files/two.txt'), 'single')
const handle = await page.$('#single')
out.handle = await attempt(handle.setInputFiles('files/one.txt'), 'single')

// Payloads and `[]`, built in the page: untrusted events, input composed, the type from the
// name when the payload names none.
out.payload = {
  ...(await attempt(
    page.setInputFiles('#multi', [
      { name: 'note.txt', mimeType: 'text/x-note', buffer: Buffer.from('hello') },
      { name: 'pic.png', buffer: Buffer.from('png') },
      { name: 'blank.txt', mimeType: '', buffer: Buffer.from('blank') },
      { name: 'noext', buffer: Buffer.from('raw') },
    ]),
  )),
  files: await stamped('multi'),
}
out.buffer = {
  ...(await attempt(
    page.setInputFiles('#single', { name: 'b.json', buffer: Buffer.from('{"a":1}') }),
  )),
  files: await stamped('single'),
}
// Upstream's server base64s a buffer with `buffer.toString('base64')`, which a plain
// Uint8Array answers with its numbers, so the page's atob refuses it.
out.uint8Array = await attempt(
  page.setInputFiles('#single', { name: 'u.bin', buffer: bytes('hi') }),
)
out.cleared = await attempt(page.setInputFiles('#multi', []), 'multi')
out.clearedEmpty = await attempt(page.setInputFiles('#empty', []), 'empty')
out.payloadIntoText = await attempt(
  page.setInputFiles('#text', { name: 'x.txt', mimeType: 'text/plain', buffer: Buffer.from('x') }),
)
out.emptyIntoText = await attempt(page.setInputFiles('#text', []))

// A directory into a webkitdirectory input: upstream sets it and waits for `input`, which
// Chromium fires only for the tab in front. Its files' mtimes are the checkout's, so they go.
out.directory = {
  ...(await attempt(page.setInputFiles('#dir', 'files/dir', { timeout: 1500 }))),
  files: (await files('dir')).map(({ lastModified, ...file }) => file),
}

// The client's errors: no prefix, nothing sent.
out.mixed = await attempt(
  page.setInputFiles('#single', ['files/one.txt', { name: 'x', mimeType: 'text/plain', buffer: Buffer.from('x') }]),
)
out.missing = await attempt(page.setInputFiles('#single', 'files/missing.txt'))
out.missingSecond = await attempt(page.setInputFiles('#multi', ['files/one.txt', 'files/nope.txt']))
out.notDirectory = await attempt(page.setInputFiles('#single', 'files/one.txt/inner'))
out.twoDirectories = await attempt(page.setInputFiles('#dir', ['files/dir', 'files/dir2']))
out.directoryAndFile = await attempt(page.setInputFiles('#dir', ['files/dir', 'files/one.txt']))
out.fileAndDirectory = await attempt(page.setInputFiles('#dir', ['files/one.txt', 'files/dir']))
out.tooLarge = await attempt(
  page.setInputFiles('#single', { name: 'big.bin', buffer: new Uint8Array(50 * 1024 * 1024) }),
)
out.noFiles = await attempt(page.setInputFiles('#single'))

// The channel validator's errors: prefixed, nothing sent.
out.noName = await attempt(page.setInputFiles('#single', { mimeType: 'text/plain', buffer: Buffer.from('x') }))
out.numberName = await attempt(page.setInputFiles('#single', { name: 7, buffer: Buffer.from('x') }))
out.numberMime = await attempt(page.setInputFiles('#single', { name: 'x', mimeType: 7, buffer: Buffer.from('x') }))
out.nullMime = await attempt(page.setInputFiles('#single', { name: 'x', mimeType: null, buffer: Buffer.from('x') }))
out.stringBuffer = await attempt(page.setInputFiles('#single', { name: 'x', buffer: 'x' }))
out.noBuffer = await attempt(page.setInputFiles('#single', { name: 'x' }))
out.numberItem = await attempt(page.setInputFiles('#multi', [{ name: 'x', buffer: Buffer.from('x') }, 5]))
out.nullItem = await attempt(page.setInputFiles('#multi', [{ name: 'x', buffer: Buffer.from('x') }, null]))

// The server's errors, after the element is found: prefixed.
out.notInput = await attempt(page.setInputFiles('#button', 'files/one.txt'))
out.nonMultiple = await attempt(page.setInputFiles('#single', ['files/one.txt', 'files/two.txt']))
out.directoryIntoFile = await attempt(page.setInputFiles('#single', 'files/dir'))
out.fileIntoDirectory = await attempt(page.setInputFiles('#dir', 'files/one.txt'))
out.payloadIntoDirectory = await attempt(
  page.setInputFiles('#dir', { name: 'x', mimeType: 'text/plain', buffer: Buffer.from('x') }),
)
out.emptyIntoDirectory = await attempt(page.setInputFiles('#dir', []))
// Chromium refuses paths for an input that is not type=file. Upstream's locator loop retries
// that until its timeout; a handle reports it.
out.pathIntoText = await attempt(page.setInputFiles('#text', 'files/one.txt', { timeout: 1000 }))
const text = await page.$('#text')
out.handlePathIntoText = await attempt(text.setInputFiles('files/one.txt'))
out.strict = await attempt(page.locator('.pair').setInputFiles('files/one.txt'))
out.timeout = await attempt(page.setInputFiles('#nope', 'files/one.txt', { timeout: 500 }))

// Handles whose element or frame went away.
const detachable = await page.$('#detachable')
await harness.pageEval(page, "document.getElementById('detachable').remove()")
out.detachedHandle = await attempt(detachable.setInputFiles('files/one.txt'))
const inner = await page.frameLocator('#child').locator('#inner').elementHandle()
await harness.pageEval(page, "document.getElementById('child').remove()")
out.frameGone = await attempt(inner.setInputFiles('files/one.txt'))

// Neo refuses payloads totalling 10 MiB, which claw-sdk's 64 MiB heap cannot carry with room to
// spare (deviations.yaml). A byte under it, both runners land every byte.
const neoLimit = 10 * 1024 * 1024
const digest = (id) =>
  harness.pageEval(
    page,
    `(async () => {
      const file = document.getElementById(${JSON.stringify(id)}).files[0]
      let hash = 0
      for (const byte of new Uint8Array(await file.arrayBuffer())) hash = (hash * 31 + byte) >>> 0
      return { name: file.name, type: file.type, size: file.size, hash }
    })()`,
  )
out.underNeoLimit = {
  ...(await attempt(
    page.setInputFiles('#single', { name: 'big.bin', buffer: Buffer.alloc(neoLimit - 1, 'neo') }),
  )),
  file: await digest('single'),
}
out.atNeoLimit = (
  await attempt(page.setInputFiles('#single', { name: 'big.bin', buffer: Buffer.alloc(neoLimit, 'neo') }))
).outcome
return out
