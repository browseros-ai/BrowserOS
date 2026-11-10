// F6 (design D22): the nested fixture plus frame D (hang.localtest.me, a public name for
// 127.0.0.1), which spins forever from 1.5 s after its load. The ai snapshot, called with an
// 8 s timeout: whether it returns within 4 s, the buttons it printed, what it printed for
// frame D (the last top-level iframe, to the end), and whether the printed refs read back.
// nest-f1-snapshot compares the rest of the snapshot.
await page.goto(fixtures.url('nest/top.html?hang'))
await new Promise((done) => setTimeout(done, 2000))
const started = Date.now()
const settled = await harness.settle(
  page.ariaSnapshot({ mode: 'ai', timeout: 8000 }),
)
const withinFourSeconds = Date.now() - started < 4000
const snapshot = settled.returned ?? ''
const lines = snapshot.split('\n')
let hung = lines.length
while (hung > 0 && !lines[hung - 1].startsWith('  - iframe')) hung--
const read = {}
for (const [, name, ref] of snapshot.matchAll(
  /- button "([^"]+)" \[ref=([^\]]+)\]/g,
))
  read[name] = await harness.settle(
    page.locator(`aria-ref=${ref}`).textContent({ timeout: 2000 }),
  )
return {
  withinFourSeconds,
  threw: settled.threw ?? null,
  frameD: lines.slice(hung - 1).join('\n'),
  read,
}
