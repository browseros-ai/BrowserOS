// Async function body: Playwright's `page` and `context` are supplied by the caller.
// Live Hacker News scenario: reads the top five stories of the current "best" list, then opens
// each story's comments page in a new tab of `context` and leaves those tabs open. The list is
// live, so check the result's internal consistency, never particular stories.

const BEST_URL = 'https://news.ycombinator.com/best'
const TOP = 5

context.setDefaultTimeout(15_000)
context.setDefaultNavigationTimeout(30_000)

// Fails on an HTTP error (Hacker News rate-limits fast clients) rather than on a later selector.
const load = async (tab, url) => {
  const response = await tab.goto(url)
  if (!response?.ok()) {
    const answer = response ? `HTTP ${response.status()}` : 'no response'
    throw new Error(`Loading ${url} failed: ${answer}.`)
  }
}
// Story links are absolute; the site's own links are relative to its root (`item?id=…`).
const absolute = (href) =>
  /^https?:\/\//.test(href) ? href : `https://news.ycombinator.com/${href.replace(/^\//, '')}`

await load(page, BEST_URL)

// One `tr.athing` row per story, its id the story's id. The row below it holds the score,
// author, age and the comments link: "N comments", or "discuss" before the first comment.
const rows = page.locator('tr.athing')
await rows.first().waitFor()
const listed = await rows.count()
if (listed < TOP) {
  throw new Error(`Expected at least ${TOP} stories on ${BEST_URL}, found ${listed}.`)
}

const stories = []
for (let i = 0; i < TOP; i++) {
  const row = rows.nth(i)
  const id = await row.getAttribute('id')
  const titleLink = row.locator('.titleline > a')
  const commentsLink = page
    .locator(`tr.athing[id="${id}"] + tr .subtext`)
    .getByRole('link', { name: /^\d+\s+comments?$|^discuss$/ })
  stories.push({
    rank: Number.parseInt(await row.locator('.rank').textContent(), 10),
    id,
    title: (await titleLink.textContent()).trim(),
    url: absolute(await titleLink.getAttribute('href')),
    comments: Number.parseInt(await commentsLink.textContent(), 10) || 0,
    commentsUrl: absolute(await commentsLink.getAttribute('href')),
  })
}

// One new tab per story, in rank order; `page` stays on the list.
const tabs = []
for (const story of stories) {
  const tab = await context.newPage()
  tabs.push(tab)
  await load(tab, story.commentsUrl)
}

// An item page repeats its story in `table.fatitem`, above one `tr.comtr` row per comment shown.
const contextPages = context.pages()
const commentTabs = []
for (const [i, tab] of tabs.entries()) {
  commentTabs.push({
    rank: stories[i].rank,
    id: stories[i].id,
    url: tab.url(),
    title: await tab.title(),
    storyTitle: (await tab.locator('.fatitem .titleline > a').textContent()).trim(),
    commentRows: await tab.locator('tr.comtr').count(),
    open: contextPages.includes(tab),
  })
}

return {
  best: { url: page.url(), title: await page.title() },
  stories,
  commentTabs,
}
