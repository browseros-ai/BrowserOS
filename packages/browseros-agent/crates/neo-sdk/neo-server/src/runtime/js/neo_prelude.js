// Neo's task API and CAPTCHA solver, supplied as the agent's `neo` parameter. Task handles
// live for one run; their pages are the Playwright provider's shared Page objects.

;(() => {
  const { provide, refuse } = __neo

  provide('neo', (_init, { context }) => {
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

    return Object.freeze({
      createTask: async (name) =>
        Task._from(
          await invoke('neo.createTask', name === undefined ? {} : { name }),
        ),
      task: async (id) => Task._from(await invoke('neo.task', { id })),
      // Every CAPTCHA verdict, unsolved included, remains a value to branch on.
      solveCaptcha: (page) => invoke('neo.solveCaptcha', { page }),
    })
  })
})()
