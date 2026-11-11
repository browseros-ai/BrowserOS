// Chromium executes a JavaScript URL in the current document before reporting ERR_ABORTED.
// Both page.goto and the main frame form must preserve the script's effect and the error.
await page.goto(fixtures.url('child.html'))
const outcomes = []
for (const target of [page, page.mainFrame()]) {
  for (const url of [
    "javascript:void(document.title = 'ran')",
    " JavaScript:void(document.title = 'ran')",
    "java\tscript:void(document.title = 'ran')",
  ]) {
    await harness.pageEval(page, "document.title = 'before'")
    outcomes.push({
      outcome: await harness.settle(target.goto(url)),
      title: await harness.pageEval(page, 'document.title'),
    })
  }
}
return outcomes
