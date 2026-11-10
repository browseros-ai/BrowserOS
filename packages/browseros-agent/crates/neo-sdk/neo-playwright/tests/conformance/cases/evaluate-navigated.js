// page.evaluate, locator.evaluate and page.waitForFunction still pending when the page leaves
// for another document: on another site, in another renderer, and on its own site. Chromium
// cuts the pending call off. A script may have run, so the evaluates fail; a wait goes on in
// the new document, where its predicate settles. Each step waits for the new document before
// the next starts.
const keep = (work) => harness.settle(work)
const pending = (url) => {
  setTimeout(() => { location.href = url }, 300)
  return new Promise(() => {})
}
const sites = { crossSite: 'b', sameSite: 'a' }
const outcome = {}
for (const [label, site] of Object.entries(sites)) {
  const moved = fixtures.url(`child.html?name=Moved-${label}`, site)
  await page.goto(fixtures.url('reads.html'))
  const evaluated = await keep(page.evaluate(pending, moved))
  await page.waitForURL(moved)
  await page.goto(fixtures.url('reads.html'))
  const element = await keep(page.locator('h1').evaluate((h1, url) => {
    setTimeout(() => { location.href = url }, 300)
    return new Promise(() => {})
  }, moved))
  await page.waitForURL(moved)
  await page.goto(fixtures.url('reads.html'))
  await harness.pageEval(
    page,
    `setTimeout(() => { location.href = ${JSON.stringify(moved)} }, 400), true`,
  )
  const waited = await keep(
    page
      .waitForFunction(() =>
        document.getElementById('name')?.textContent?.startsWith('Moved-')
          ? [location.hostname, document.getElementById('name').textContent]
          : new Promise(() => {}),
      )
      .then((handle) => handle.jsonValue()),
  )
  outcome[label] = { evaluated, element, waited }
}
return outcome
