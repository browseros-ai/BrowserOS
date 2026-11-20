// Neo's task API, CAPTCHA solver and Jev helpers, supplied as the agent's `neo` parameter. Task
// handles live for one run; their pages are the Playwright provider's shared Page objects.
//
// The Jev helpers (`neo.find`, `neo.findAll`, `neo.check`, `neo.choose`, `neo.stop`) do all their
// page work here: which tab to look at, the snapshot, the re-look loop and the stop. The Rust
// built-ins (builtins/jev_verbs.rs) only take snapshot text and answer a verdict value:
//
//   neo.find(page, ask) ─▶ tab to look at (page, or its newest popup this run)
//     ─▶ ariaSnapshot ─▶ invoke('neo.find', { url, title, snapshot, ask, role, picked })
//     ─▶ found: tab.getByRef(ref)   not_found: look again every ~300 ms until `timeout`
//        ambiguous / deadline: throw stop(…) (optional: null)
//
// A helper that is not sure stops the script instead of guessing (Jev helpers v1 design, §1 "A
// stop"): `stop` writes its console line and returns the bootstrap's branded error, which the
// door records as a refused op (code `stopped`) and which, left uncaught, `run` (bootstrap.js)
// turns into the run's value `{ stopped: Stop }`. A stop sends nothing to the page and retries
// nothing. Every call writes one short `console.info` line, so the run's reply shows the agent
// each step that was done.

;(() => {
  const { provide, refuse, stopError } = __neo

  provide('neo', (_init, { context, console, setTimeout }) => {
    const tasks = new Map()

    // Every task operation settles as the Playwright provider's own operations do: before its
    // promise settles, answered or rejected, the context reads every tree that went stale
    // (`BrowserContext._settleFrames`), the tree of a page this operation handed out or bound
    // the built-in `page` to included. So `frames()` is the page's real tree once the call
    // resolves. A failed read is the settle step's own business and never fails the call.
    const invoke = async (name, input) => {
      try {
        return await __neo.invoke(name, input)
      } finally {
        await context._settleFrames().catch(() => {})
      }
    }

    class Task {
      #id
      #ordered
      #added = []
      #closed = false

      constructor(id, pages) {
        this.#id = id
        this.#ordered = pages
      }

      get id() {
        return this.#id
      }

      pages() {
        if (this.#closed) return []
        // Keep the server's order, then the context's newly learned pages (including popups).
        // Each tab counts once, as the context's Page for it now: when the built-in `page`
        // lands on a tab this task already listed, it takes over that tab's entry.
        const pages = new Map()
        for (const listed of [
          ...this.#ordered,
          ...context._knownPages().filter((page) => page.taskId === this.#id),
          ...this.#added,
        ]) {
          const page = context._pageFromResult(listed) ?? listed
          if (!pages.has(page.tabId)) pages.set(page.tabId, page)
        }
        return [...pages.values()].filter((page) => !page.isClosed())
      }

      page(tabId) {
        this.#requireOpen('task.page')
        const pages = this.pages()
        const page = pages.find((page) => page.tabId === tabId)
        if (page) return page
        throw refuse(
          'task.page',
          'task_page_missing',
          `task ${this.#id} has no tab ${tabId}; its open tab ids: ${pages.map((page) => page.tabId).join(', ') || 'none'}`,
        )
      }

      async newPage() {
        this.#requireOpen('task.newPage')
        const { page } = await invoke('task.newPage', { task: this.#id })
        this.#added.push(page)
        return page
      }

      async close() {
        this.#requireOpen('task.close')
        await invoke('task.close', { task: this.#id })
        this.#closed = true
        this.#ordered = []
        this.#added = []
      }

      static _from({ id, pages }) {
        let task = tasks.get(id)
        if (task) task.#ordered = pages
        else {
          task = new Task(id, pages)
          tasks.set(id, task)
        }
        return task
      }

      #requireOpen(api) {
        if (!this.#closed) return
        throw refuse(
          api,
          'task_not_found',
          `no task \`${this.#id}\` is open in Neo (tasks live in memory and end with \`task.close()\`; a restart keeps only tasks with open tabs). Start one with \`neo.createTask(name)\`, or find your tabs in \`context.pages()\` and read \`page.taskId\`.`,
        )
      }
    }

    const jev = jevVerbs({ context, console, setTimeout, refuse, stopError })

    return Object.freeze({
      createTask: async (name) =>
        Task._from(
          await invoke('neo.createTask', name === undefined ? {} : { name }),
        ),
      task: async (id) => Task._from(await invoke('neo.task', { id })),
      // Every CAPTCHA verdict, unsolved included, remains a value to branch on.
      solveCaptcha: (page) => invoke('neo.solveCaptcha', { page }),
      find: jev.find,
      findAll: jev.findAll,
      check: jev.check,
      choose: jev.choose,
      stop: jev.stop,
    })
  })

  // How often `find` and a timed `check` look at the page again, and `find`'s default patience.
  const RELOOK_MS = 300
  const FIND_TIMEOUT_MS = 5000
  // The snapshot cap the helpers ask for: high enough that Rust, not the snapshot, decides what
  // is too big for Jev (contract decision 5), so nothing is cut silently before Jev sees it.
  const SNAPSHOT_TOKENS = 250000
  // How many lookalikes a stop hands back, best first.
  const LOOKALIKES = 3
  // How many refs a `findAll` console line names before it says how many more there are.
  const LISTED_REFS = 6
  // The built-in's transport faults that stop the script, as the stop's `why`. Any other failure
  // (`jev_unavailable`: no key; a cancelled run) stays the op's error.
  const FAULT_STOPS = new Map([
    ['jev_too_big', 'too_big'],
    ['jev_down', 'jev_down'],
  ])

  /** The Jev helpers over one run's state: its earlier picks and its tab follows. */
  function jevVerbs({ context, console, setTimeout, refuse, stopError }) {
    // This run's earlier `find` picks per tab, so Jev can flag a ref picked again under another
    // ask. Per tab because a ref is unique only within its tab: two tabs can both print `e1`.
    const picks = new Map()
    // Per page the script passed: the tabs followed from it, oldest first (a popup's popup still
    // counts as opened from the script's page), and the follows not yet written to the console.
    const lineage = new Map()
    // Each new tab's opener, read once: an opener never changes.
    const openers = new WeakMap()
    // The tabs whose popups the helpers listen for.
    const watched = new WeakSet()

    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
    const quote = (text) => JSON.stringify(String(text))
    const p2 = (p) => Number(p).toFixed(2)

    const note = (line) => console.info(line)

    async function openerOf(page) {
      if (openers.has(page)) return openers.get(page)
      let opener = null
      try {
        opener = await page.opener()
      } catch {
        // A tab that closed or never answered is no one's popup for this call.
      }
      openers.set(page, opener)
      return opener
    }

    /**
     * Listens for `tab`'s popups. A click does not tell the realm about the tab it opened: the
     * context learns a popup only from the events pump. While a `popup` listener is on, Neo holds
     * an op's reply until the pump has handed over the events it caused (events.ts, the pump
     * ledger), so by the time the script's click resolves, the popup is in its tab's line and
     * the next helper looks at it. A popup opened before the first helper on that tab is caught
     * by the fallback scan in `lookAt` instead, once the context knows the tab.
     */
    function watch(tab) {
      if (watched.has(tab) || typeof tab.on !== 'function') return
      watched.add(tab)
      tab.on('popup', (popup) => {
        for (const line of lineage.values()) {
          if (!line.tabs.includes(tab) || line.tabs.includes(popup)) continue
          line.tabs.push(popup)
          line.pending.push({ tab: popup, opener: tab })
        }
        watch(popup)
      })
    }

    function lineOf(page) {
      let line = lineage.get(page)
      if (line === undefined) {
        line = { tabs: [page], pending: [] }
        lineage.set(page, line)
      }
      watch(page)
      return line
    }

    // The line's script page, and the tabs followed from it that are still open.
    const openTabs = (line) =>
      line.tabs.filter((tab, at) => at === 0 || !tab.isClosed())

    /**
     * The fallback: tabs the context knows (an event, a result, `context.pages()`), first seen
     * after the line's newest tab, whose opener is in the line. It follows the whole chain: a
     * popup B opened from popup A joins once A has, so it scans again after each follow until
     * nothing joins. One `page.opener` op per new tab, cached.
     */
    async function scanKnown(line) {
      for (;;) {
        const known = context._knownPages()
        const newer = known
          .slice(known.indexOf(openTabs(line).at(-1)) + 1)
          .filter(
            (tab) =>
              tab._newInRun && !tab.isClosed() && !line.tabs.includes(tab),
          )
          .reverse()
        let followed = false
        for (const tab of newer) {
          const opener = await openerOf(tab)
          if (opener === null || !line.tabs.includes(opener)) continue
          line.tabs.push(tab)
          line.pending.push({ tab, opener })
          watch(tab)
          followed = true
          break
        }
        if (!followed) return
      }
    }

    /**
     * The newest open tab of `line` as it stands, with no scan: synchronous, for `neo.stop`.
     * Each follow is one console line, written by the first call to see it.
     */
    function newestTab(line, api) {
      for (const { tab, opener } of line.pending.splice(0)) {
        if (tab.isClosed()) continue
        note(
          `${api} follows the new tab ${tab.url()} (opened from ${opener.url()})`,
        )
      }
      return openTabs(line).at(-1)
    }

    /**
     * The tab a call on `page` looks at: the newest open tab opened from `page` (or from a tab
     * already followed from it) during this run, else `page`.
     */
    async function lookAt(page, api) {
      const line = lineOf(page)
      await scanKnown(line)
      return newestTab(line, api)
    }

    function requirePage(api, page) {
      if (
        page === null ||
        typeof page !== 'object' ||
        typeof page.ariaSnapshot !== 'function'
      )
        throw refuse(
          api,
          'invalid_input',
          `${api}: the first argument must be a page`,
        )
    }

    function requireText(api, name, value) {
      if (typeof value !== 'string' || value.trim() === '')
        throw refuse(
          api,
          'invalid_input',
          `${api}: \`${name}\` must be a non-empty string`,
        )
    }

    function timeoutOf(api, value, fallback) {
      if (value === undefined) return fallback
      if (typeof value !== 'number' || !Number.isFinite(value) || value < 0)
        throw refuse(
          api,
          'invalid_input',
          `${api}: \`timeout\` must be a number of ms >= 0`,
        )
      return value
    }

    /**
     * The stop's `next` line: how to carry on by hand from the tab the helper looked at, in a
     * later run too. A Page in `context.pages()` carries its `tabId`, and the agent's code runs
     * inside a block of its own, so `const page` there shadows the `page` parameter.
     */
    function nextLine(tab, hasLookalikes) {
      const act = hasLookalikes
        ? 'Act on a lookalike with page.getByRef(ref), or look with page.ariaSnapshot().'
        : 'Look with page.ariaSnapshot() and act on its refs.'
      if (tab === null)
        // Only `neo.stop` before the run's page opened its tab: nothing happened on any tab.
        return 'Not an error; nothing was undone. Carry on by hand: look with page.ariaSnapshot() and act on its refs.'
      return `Not an error; nothing was undone. Carry on by hand on this tab: const page = context.pages().find(p => p.tabId === ${tab}). ${act}`
    }

    /**
     * The one way a helper ends the script. Callers `throw stop(…)`. It writes the stop's console
     * line and returns the bootstrap's branded error carrying the `Stop` record (agent.d.ts):
     * where (`at`, the agent's line captured at the call's entry), why, the tab looked at, up to
     * three lookalikes with refs that work with `getByRef` on that tab, the `next` line, and
     * `log`, the run's console lines so far (bootstrap.js `consoleTail`), for hosts that show the
     * model only the value.
     */
    function stop(api, { site, ask, why, tab, picks = [], obstacle, detail }) {
      const lookalikes = picks
        .slice(0, LOOKALIKES)
        .map(({ ref, label, p }) => ({
          ref,
          is: label,
          p: Math.round(p * 100) / 100,
        }))
      const tabId = tab.tabId ?? null
      const record = {
        at: site,
        call: api,
        ask,
        why,
        tab: tabId,
        url: tab.url(),
      }
      if (obstacle !== undefined && obstacle !== 'none')
        record.obstacle = obstacle
      record.lookalikes = lookalikes
      if (detail !== undefined) record.detail = detail
      record.next = nextLine(tabId, lookalikes.length > 0)
      note(
        `${api} ${quote(ask)} -> stopped: ${why}` +
          lookalikes.map((l) => ` · ${l.ref} ${p2(l.p)}`).join(''),
      )
      // Read after the stop's own line, so the log ends with it.
      record.log = __neo.consoleTail()
      return stopError(api, record)
    }

    /** One look: the tab's snapshot and where it was taken. */
    async function snapshot(tab, text) {
      const options = { maxTokens: SNAPSHOT_TOKENS }
      if (text) options.text = true
      const taken = await tab.ariaSnapshot(options)
      return { url: tab.url(), title: await tab.title(), snapshot: taken }
    }

    /**
     * Asks the Rust built-in. Jev down, or a page over Jev's limit, stops the script: the
     * built-in's op keeps its fault in the trace, and the stop's `detail` is its message.
     */
    async function decide(name, input, { site, said, tab }) {
      try {
        return await __neo.invoke(name, input, { site })
      } catch (error) {
        const why = FAULT_STOPS.get(error?.code)
        if (why === undefined) throw error
        throw stop(name, { site, ask: said, why, tab, detail: error.message })
      }
    }

    const obstacleLine = (obstacle) =>
      obstacle && obstacle !== 'none' ? ` · obstacle ${obstacle}` : ''

    /**
     * Looks at the page until `done(verdict)` or `timeout` (ms; 0 looks once), every
     * `RELOOK_MS`. An unchanged page is not asked again: Jev would answer the same. `input(tab)`
     * is the ask's own fields for the tab this look read, which a popup can change mid-loop.
     */
    async function lookUntil({
      api,
      page,
      text,
      timeout,
      input,
      site,
      said,
      done,
    }) {
      const started = Date.now()
      let verdict = null
      let tab = page
      let last = null
      for (;;) {
        tab = await lookAt(page, api)
        const seen = await snapshot(tab, text)
        const fingerprint = `${seen.url}\n${seen.snapshot}`
        if (verdict === null || fingerprint !== last) {
          last = fingerprint
          verdict = await decide(
            api,
            { ...seen, ...input(tab) },
            { site, said, tab },
          )
        }
        if (done(verdict)) break
        if (Date.now() - started + RELOOK_MS > timeout) break
        await sleep(RELOOK_MS)
      }
      return { verdict, tab }
    }

    /** The earlier picks on `tab`, the list `find` adds its pick to. */
    function pickedOn(tab) {
      const key = tab.tabId ?? tab
      let picked = picks.get(key)
      if (picked === undefined) {
        picked = []
        picks.set(key, picked)
      }
      return picked
    }

    const findInput = (askText, role) => (tab) => {
      const input = { ask: askText, picked: pickedOn(tab) }
      if (role !== undefined) input.role = role
      return input
    }

    async function find(page, askText, options = {}) {
      const api = 'neo.find'
      const site = __neo.callSite()
      requirePage(api, page)
      requireText(api, 'ask', askText)
      const timeout = timeoutOf(api, options?.timeout, FIND_TIMEOUT_MS)
      const head = `${api} ${quote(askText)}`
      const { verdict, tab } = await lookUntil({
        api,
        page,
        text: false,
        timeout,
        input: findInput(askText, options?.role),
        site,
        said: askText,
        done: (verdict) => verdict.outcome !== 'not_found',
      })
      if (verdict.outcome === 'found') {
        const { pick } = verdict
        note(
          `${head} -> ${pick.ref} ${pick.label} p${p2(pick.p)}${obstacleLine(verdict.obstacle)}`,
        )
        pickedOn(tab).push({ ref: pick.ref, ask: askText })
        return tab.getByRef(pick.ref)
      }
      if (options?.optional === true) {
        note(`${head} -> null (${verdict.outcome})`)
        return null
      }
      // neo-jev's gate answers `ambiguous` for a clear pick of an element this run already
      // picked under another ask; that element leads the lookalikes.
      const reused = verdict.flags.reused
      throw stop(api, {
        site,
        ask: askText,
        why: verdict.outcome,
        tab,
        picks: verdict.lookalikes,
        obstacle: verdict.obstacle,
        detail: reused
          ? `${verdict.lookalikes[0]?.ref ?? 'the best fit'} is already picked for ${quote(reused)}`
          : undefined,
      })
    }

    async function findAll(page, askText, options = {}) {
      const api = 'neo.findAll'
      const site = __neo.callSite()
      requirePage(api, page)
      requireText(api, 'ask', askText)
      const { verdict, tab } = await lookUntil({
        api,
        page,
        text: false,
        timeout: 0,
        input: findInput(askText, options?.role),
        site,
        said: askText,
        done: () => true,
      })
      // Elements Jev cannot call either way: a list missing one, or holding one too many, would
      // be a silent wrong answer, so the script stops on them instead.
      if (verdict.unsure.length > 0)
        throw stop(api, {
          site,
          ask: askText,
          why: 'unsure',
          tab,
          picks: verdict.unsure,
          obstacle: verdict.obstacle,
        })
      const refs = verdict.picks.map((pick) => pick.ref)
      const more = refs.length - LISTED_REFS
      const shown = refs.length
        ? `${refs.slice(0, LISTED_REFS).join(', ')}${more > 0 ? ` and ${more} more` : ''}`
        : 'none'
      note(
        `${api} ${quote(askText)} -> ${shown}${obstacleLine(verdict.obstacle)}`,
      )
      return refs.map((ref) => tab.getByRef(ref))
    }

    async function check(page, claim, options = {}) {
      const api = 'neo.check'
      const site = __neo.callSite()
      requirePage(api, page)
      requireText(api, 'claim', claim)
      const { verdict, tab } = await lookUntil({
        api,
        page,
        text: true,
        timeout: timeoutOf(api, options?.timeout, 0),
        input: () => ({ claim }),
        site,
        said: claim,
        // A timed check waits for a clear yes: an unsure one may still settle.
        done: (verdict) => verdict.yes && !verdict.unsure,
      })
      if (verdict.unsure)
        throw stop(api, {
          site,
          ask: claim,
          why: 'unsure',
          tab,
          obstacle: verdict.obstacle,
          detail: `p ${p2(verdict.p)}`,
        })
      note(
        `${api} ${quote(claim)} -> ${verdict.yes ? 'yes' : 'no'} p${p2(verdict.p)}` +
          obstacleLine(verdict.obstacle),
      )
      return verdict.yes
    }

    async function choose(page, question, options) {
      const api = 'neo.choose'
      const site = __neo.callSite()
      requirePage(api, page)
      requireText(api, 'question', question)
      if (
        !Array.isArray(options) ||
        options.length === 0 ||
        options.some((o) => typeof o !== 'string')
      )
        throw refuse(
          api,
          'invalid_input',
          `${api}: \`options\` must be a non-empty array of strings`,
        )
      const { verdict, tab } = await lookUntil({
        api,
        page,
        text: true,
        timeout: 0,
        input: () => ({ question, options }),
        site,
        said: question,
        done: () => true,
      })
      if (!verdict.clear)
        throw stop(api, {
          site,
          ask: question,
          why: 'unsure',
          tab,
          obstacle: verdict.obstacle,
          detail: verdict.ranked
            .slice(0, 2)
            .map(([option, p]) => `${quote(option)} ${p2(p)}`)
            .join(' · '),
        })
      note(
        `${api} ${quote(question)} -> ${quote(verdict.choice)} p${p2(verdict.p)}` +
          obstacleLine(verdict.obstacle),
      )
      return verdict.choice
    }

    /**
     * `neo.stop(page, why)`: the script's own "not what I expected", with the same hand-back as
     * a helper's stop. Synchronous, so it takes the newest tab already known in `page`'s line and
     * scans nothing. The brand is refused at the door under any name, so the registered
     * `neo.stop` method (builtins/jev_verbs.rs, for its listing) is never run.
     */
    function stopHere(page, why) {
      const api = 'neo.stop'
      const site = __neo.callSite()
      requirePage(api, page)
      requireText(api, 'why', why)
      const line = lineage.get(page)
      const tab = line === undefined ? page : newestTab(line, api)
      throw stop(api, { site, ask: why, why: 'script', tab })
    }

    return { find, findAll, check, choose, stop: stopHere }
  }
})()
