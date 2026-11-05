// F7 (design D22): refs into frames that changed after the snapshot. a1 navigates (taking b2
// and b3 with it), then b1 is removed (taking c2, a3 and c4); each old ref is read with a 1 s
// timeout, and the top frame's ref still reads.
await page.goto(fixtures.url('nest/top.html'))
let snapshot = ''
for (let attempt = 0; attempt < 20 && !snapshot.includes('C4 button'); attempt++)
  snapshot = await page.ariaSnapshot({ mode: 'ai' })
const refs = Object.fromEntries(
  [...snapshot.matchAll(/- button "([^"]+) button" \[ref=([^\]]+)\]/g)].map(
    ([, name, ref]) => [name, ref],
  ),
)
const read = (name) =>
  harness.settle(
    page.locator(`aria-ref=${refs[name]}`).textContent({ timeout: 1000 }),
  )
await harness.pageEval(
  page,
  "document.querySelector('iframe[title=a1]').src = '/nest/b3.html?navigated'",
)
await harness.poll(
  page,
  "document.querySelector('iframe[title=a1]').contentDocument?.querySelector('button')?.textContent === 'B3 button'",
)
const navigated = { A1: await read('A1'), B2: await read('B2'), B3: await read('B3') }
await harness.pageEval(page, "document.querySelector('iframe[title=b1]').remove()")
const removed = { B1: await read('B1'), C4: await read('C4') }
return { navigated, removed, top: await read('Top') }
