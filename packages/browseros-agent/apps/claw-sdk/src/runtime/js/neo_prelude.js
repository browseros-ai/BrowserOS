// Neo's own methods: the agent's `neo` parameter. Kept apart from the page prelude because
// sdk-playwright's prelude replaces that file, and these stay.
//
//   neo.solveCaptcha(page) -> { verdict, type, reason, message, durationMs, paidTasks }
//
// Every verdict, unsolved included, is a value to branch on; a refusal (another solve or write
// on the page, a closed page, no provider key) throws a NeoError like any SDK call.

;(() => {
  const { invoke, provide } = __neo

  const neo = Object.freeze({
    solveCaptcha: (page) => invoke('neo.solveCaptcha', { page }),
  })

  provide('neo', () => neo)
})()
