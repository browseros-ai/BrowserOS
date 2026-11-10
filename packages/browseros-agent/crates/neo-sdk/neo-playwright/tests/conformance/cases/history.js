// goBack, goForward and reload: each one's response, and where the page is after it. The
// pages are history.html, which stays out of the back/forward cache (see its comment).
const step = async (work) =>
  harness.settle(
    work.then(async (response) => ({
      response:
        response === null
          ? null
          : { status: response.status(), url: response.url() },
      url: page.url(),
      name: await harness.pageEval(
        page,
        "document.getElementById('name')?.textContent ?? null",
      ),
    })),
  )
const steps = {}
steps.backAtStart = await step(page.goBack())
await page.goto(fixtures.url('history.html?name=One'))
await page.goto(fixtures.url('history.html?name=Two'))
steps.back = await step(page.goBack())
steps.forward = await step(page.goForward())
steps.forwardAtEnd = await step(page.goForward())
await harness.pageEval(page, "document.getElementById('name').textContent = 'Edited'")
steps.reload = await step(page.reload())
steps.reloadAtDomContentLoaded = await step(
  page.reload({ waitUntil: 'domcontentloaded' }),
)
return steps
