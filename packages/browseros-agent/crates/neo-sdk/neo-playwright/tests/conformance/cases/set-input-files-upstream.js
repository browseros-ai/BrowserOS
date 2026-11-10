// What upstream's page-set-input-files.spec.ts checks of setInputFiles that tier 2 cannot run
// yet: those tests read back with $eval or locator.evaluate (R10c), or need Node, upstream's
// test server or exposeFunction. Here the page is read with harness.pageEval instead.
const read = (element) =>
  harness.pageEval(
    page,
    `(async () => Promise.all([...(${element}).files].map(async (f) => ({
      name: f.name, type: f.type, size: f.size, text: f.size <= 64 ? await f.text() : null,
    }))))()`,
  )
const out = {}

// "input event.composed should be true and cross shadow dom boundary": a payload into an input
// in an open shadow root. `input` is composed and reaches the body; `change` stays inside.
await page.goto(fixtures.url('files-shadow.html'))
const shadowInput = "document.querySelector('div').shadowRoot.querySelector('input')"
out.shadow = {
  outcome: await harness.settle(
    page.locator('input').setInputFiles({
      name: 'test.txt',
      mimeType: 'text/plain',
      buffer: Buffer.from('this is a test'),
    }),
  ),
  firedEvents: await harness.pageEval(page, 'window.firedEvents'),
  firedBodyEvents: await harness.pageEval(page, 'window.firedBodyEvents'),
  files: await read(shadowInput),
}

// "should set from memory", by the page form with a selector.
await page.goto(fixtures.url('files.html'))
out.memory = {
  outcome: await harness.settle(
    page.setInputFiles('#single', {
      name: 'test.txt',
      mimeType: 'text/plain',
      buffer: Buffer.from('this is a test'),
    }),
  ),
  events: await harness.pageEval(page, 'window.__takeEvents()'),
  files: await read("document.getElementById('single')"),
}

// "input should trigger events when files changed second time" (upstream #20079): a second
// path into the same multiple input fires input and change again.
const multi = page.locator('#multi')
const upload = async (path) => ({
  outcome: await harness.settle(multi.setInputFiles(path)),
  events: (await harness.pageEval(page, 'window.__takeEvents()')).map((e) => e.type),
  files: await read("document.getElementById('multi')"),
})
out.firstUpload = await upload('files/one.txt')
out.secondUpload = await upload('files/photo.png')

// "should upload a file after popup" (upstream #29923): a popup opened with a user gesture and
// closed, then a path through an element handle.
await page.goto(fixtures.url('upstream/input/fileupload.html'))
const popup = page.waitForEvent('popup', { timeout: 3000 })
await harness.pageEval(page, "(window.__popup = window.open('about:blank'), true)")
await (await popup).close()
const input = await page.$('input')
out.afterPopup = {
  outcome: await harness.settle(input.setInputFiles('files/one.txt')),
  files: await read("document.querySelector('input')"),
}

// "should work with CSP": default-src 'none' is in force (an inline script is refused), and a
// path and a payload both land.
await page.goto(fixtures.url('files-csp.html'))
out.csp = {
  inlineScript: await harness.pageEval(
    page,
    `new Promise((done) => {
      document.addEventListener('securitypolicyviolation', (e) => done(e.effectiveDirective))
      const script = document.createElement('script')
      script.textContent = 'window.ran = true'
      document.head.append(script)
      setTimeout(() => done(window.ran ? 'ran' : 'no violation'), 1000)
    })`,
  ),
  path: await harness.settle(page.setInputFiles('#path', 'files/one.txt')),
  payload: await harness.settle(
    page.setInputFiles('#payload', { name: 'x.txt', mimeType: 'text/plain', buffer: Buffer.from('xyz') }),
  ),
  files: {
    path: await read("document.getElementById('path')"),
    payload: await read("document.getElementById('payload')"),
  },
}
return out
