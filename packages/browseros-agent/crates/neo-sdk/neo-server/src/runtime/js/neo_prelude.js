// Neo's task API and CAPTCHA solver, supplied as the agent's `neo` parameter. Task handles
// live for one run; their pages are the Playwright provider's shared Page objects.

;(() => {
  const { invoke, provide, refuse } = __neo

  provide('neo', (_init, { context }) => {
    const tasks = new Map()

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
        // The shared Page cache supplies liveness and identity.
        return [
          ...new Set([
            ...this.#ordered,
            ...context.pages().filter((page) => page.taskId === this.#id),
            ...this.#added,
          ]),
        ].filter((page) => !page.isClosed())
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
          `no task \`${this.#id}\` is open in Neo (tasks live in memory and end with \`task.close()\` or a Neo restart). Start one with \`neo.createTask(name)\`, or find your tabs in \`context.pages()\` and read \`page.taskId\`.`,
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
