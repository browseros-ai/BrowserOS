// page.goto with a javascript: URL, however it is spelled: upstream sends it and Chromium runs
// the script in the current document's main world; Neo refuses it before anything is sent,
// because that script would run outside Neo's privacy sweep (decided).
await page.goto(fixtures.url('child.html'))
const outcomes = []
for (const url of [
  "javascript:void(document.title = 'ran')",
  " JavaScript:void(document.title = 'ran')",
]) {
  outcomes.push(await harness.settle(page.goto(url)))
}
return { outcomes, title: await harness.pageEval(page, 'document.title') }
