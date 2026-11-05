// The placeholder page API: the agent's `page` and `context` over the four placeholder
// built-ins (context.newPage {url?}, page.goto {page, url}, page.title {page}, page.close {page}),
// until sdk-playwright's prelude replaces this file. A Page serializes as the page brand the
// door checks, {"$neo":"page","id","tab"}; the run's own page has id and tab null until the
// first call on it opens its tab (dispatch's lazy default page).

;(() => {
  const { invoke, brands, onReply, provide } = __neo

  // Page -> { id, tab }. Kept out of the object, so the brand is the only form a page takes.
  const handles = new WeakMap()
  // Page id -> Page, so one tab is one object within a run.
  const byId = new Map()
  // Page id -> the latest fact a reply carried { url, title, closed }, for the sync accessors.
  const facts = new Map()

  class Page {
    constructor(id, tab) {
      handles.set(this, { id, tab })
    }
    toJSON() {
      const { id, tab } = handles.get(this)
      return { $neo: 'page', id, tab }
    }
    url() {
      const { id } = handles.get(this)
      return (id !== null && facts.get(id)?.url) || ''
    }
    isClosed() {
      const { id } = handles.get(this)
      return id !== null && facts.get(id)?.closed === true
    }
    goto(url) {
      return invoke('page.goto', { page: this, url })
    }
    title() {
      return invoke('page.title', { page: this })
    }
    async close() {
      await invoke('page.close', { page: this })
    }
  }

  function pageFor(id, tab) {
    let page = byId.get(id)
    if (page === undefined) {
      page = new Page(id, tab)
      byId.set(id, page)
    }
    return page
  }

  brands.register('page', (json) => pageFor(json.id, json.tab))

  // The run's page while it has no tab. The reply of the call that opened it names the tab.
  let unopened = null
  onReply((envelope) => {
    for (const fact of envelope.pages ?? []) facts.set(fact.id, fact)
    if (envelope.defaultPage !== undefined && unopened !== null) {
      const handle = handles.get(unopened)
      handle.id = envelope.defaultPage.id
      handle.tab = envelope.defaultPage.tab
      byId.set(handle.id, unopened)
      unopened = null
    }
  })

  const context = Object.freeze({
    newPage: (options) => invoke('context.newPage', options ?? {}),
  })

  provide('page', (init) => {
    if (init.page) return pageFor(init.page.id, init.page.tab)
    unopened = new Page(null, null)
    return unopened
  })
  provide('context', () => context)
})()
