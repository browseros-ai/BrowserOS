/**
 * agent.d.ts: what a script sent to Neo's MCP `run` tool can use. Give this file to your
 * agent together with Neo's MCP URL. The `run` tool's own description says how to work a page
 * (look with `page.ariaSnapshot()`, act on its refs, look again); this file declares the API and
 * holds the differences from Playwright that description leaves out.
 *
 * `code` is the body of an async function: `await` each call, `return` a JSON value.
 * Each `run` call starts with fresh JavaScript; browser tabs carry over.
 * A run has a task when the host gives it one or the script picks one. With no task,
 * built-in `page` and `context.newPage()` each open a NEW tab with no task.
 * To keep tabs across runs yourself, create a task and return its id:
 *   first run:        const task = await neo.createTask('hn top 5'); return task.id
 *   every later run:  const task = await neo.task('<that id>')
 *   when finished:    await task.close()
 * With a task (the host's, or after `neo.createTask()` or `neo.task()`), built-in `page` uses
 * that task's last-used open tab, else another open tab, else a new one. `context.newPage()` opens in the task.
 * The latest task pick wins. Popups join their opener's task. You may declare `const page`.
 * Built-in `page` keeps the first tab it gets for the whole run. `await neo.task(id)` gives it
 * the task's open tab at once, so `page.url()` is that tab's on the next line. Otherwise (no
 * task, none of its tabs open, or this run already holds that tab's Page, say from
 * `context.pages()`) it takes its tab at its first action: before that, `page.tabId` and
 * `page.taskId` are null and `page.url()` is 'about:blank'.
 * Tab ids here are short for reading; real ones are large numbers, e.g. 1157427185.
 *
 * The reply is what you return, plus console lines and existing notes. Return `task.id`
 * to keep it. Lost it? `context.pages().find(p => p.url().includes('ycombinator'))?.taskId`
 * `context.pages()` lists the tabs this run may use. When the host binds runs to their task,
 * those are only this task's tabs: the ones its runs opened, their popups, and the tabs of
 * tasks you created; any other tab is refused with `page_not_in_task`.
 * Tasks live in Neo's memory and end with `task.close()`. A task with open tabs comes back
 * after a restart of Neo's server; a browser restart ends them all.
 */

// A hand-out, not application code: no workspace tsconfig includes it, so these globals
// never leak into the apps. It type-checks on its own against Playwright's types.
/// <reference types="node" />
import type { Expect } from '@playwright/test'
import type * as pw from 'playwright-core'

declare module 'playwright-core' {
  /** The options Neo's `ariaSnapshot()` takes beyond Playwright's. */
  interface NeoAriaSnapshotOptions {
    /** Left out: Neo's short list of what can be acted on, with refs. `'full'`: Playwright's own
     * text with a ref on every visible element, capped at `maxTokens`. */
    mode?: 'ai' | 'default' | 'full'
    /** `true`: always include the page's text; `false`: never. Left out: only while the whole
     * stays under about 3,000 tokens, and a line at its top says when it was left out. */
    text?: boolean
    /** Print each link's url. */
    urls?: boolean
    /** The cap, about 8,000 tokens by default. A `# cut:` line says what was left out and which
     * section to snapshot instead; a block with a dialog, alertdialog, alert or status role past
     * the cut is still printed whole. */
    maxTokens?: number
    /** On a page: only what changed since your last whole-page snapshot of this tab. */
    delta?: boolean
    /** `true`: the snapshot exactly as it was before Neo left out what the browser has made inert,
     * such as the page behind a modal dialog. Left out: a `# not listed:` line at the top says
     * how much was left out and names this option. */
    inert?: boolean
    timeout?: number
  }
  interface Locator {
    ariaSnapshot(options?: NeoAriaSnapshotOptions): Promise<string>
    /** Neo-only. Strictly resolves one attached element and returns its node-bound ref.
     * Reuses an engine ref or mints n<k>, prefixed f<seq> in other documents. Keep the ref
     * with its task's page for later runs. Removal or navigation makes it stale; minting
     * changes neither snapshots nor actionability checks. */
    ref(options?: { timeout?: number }): Promise<string>
  }
  /** Playwright's Page, plus: */
  interface Page {
    ariaSnapshot(options?: NeoAriaSnapshotOptions): Promise<string>
    /** The tab's id. It stays the same across runs. `null` only for the built-in `page`
     * before it has a tab (see `page`). */
    readonly tabId: number | null
    /** The tab's task; `null` when it has no task (or for the built-in `page` before it has
     * a tab). */
    readonly taskId: string | null
  }
}

declare global {
  /** Your own set of tabs for one piece of work, such as one user request. */
  interface Task {
    /** Pass this to `neo.task()` in later runs, e.g. "hn-top-5-3f9a". */
    readonly id: string
    /** This task's tabs, oldest first (popups from its tabs included). */
    pages(): Page[]
    /** One of this task's tabs, by tab id. Throws `task_page_missing` if it has no such tab. */
    page(tabId: number): Page
    /** Open a new tab in this task. */
    newPage(): Promise<Page>
    /** Close all of this task's tabs and end the task. */
    close(): Promise<void>
  }

  /** Playwright 1.64's Page plus `tabId` and `taskId`. */
  type Page = pw.Page

  /** Why and where a helper stopped the script. Uncaught, it is the run's result: `{ stopped: Stop }`. */
  interface Stop {
    /** The script line that stopped, e.g. `run:12:9`. */
    at: string
    call: 'neo.find' | 'neo.findAll' | 'neo.check' | 'neo.choose' | 'neo.stop'
    /** The ask, claim or question; for `neo.stop`, your own words. */
    ask: string
    /** `not_found`: nothing fits, after `timeout`. `ambiguous`: two or more fit, or the best fit was
     * already picked for another ask. `unsure`: Jev cannot tell. `too_big`: the page is over Jev's
     * limit; do this step by hand. `jev_down`: Jev did not answer. `script`: `neo.stop`. */
    why:
      | 'not_found'
      | 'ambiguous'
      | 'unsure'
      | 'too_big'
      | 'jev_down'
      | 'script'
    /** The tab the helper looked at (the tab your page opened, when it followed one) and its address.
     * `null` only when the page never got a tab. */
    tab: number | null
    url: string
    /** What is in the way, when Jev saw something. */
    obstacle?:
      | 'captcha'
      | 'login_wall'
      | 'cookie_banner'
      | 'error_page'
      | 'modal'
    /** Up to three nearest elements, best first. Each `ref` works with `getByRef` on tab `tab`, in this
     * run and later ones, until its element goes or the tab navigates. Empty for check, choose and stop. */
    lookalikes: { ref: string; is: string; p: number }[]
    /** One more fact when there is one, e.g. `e180 is already picked for "the State dropdown"`. */
    detail?: string
    /** What ran and what you logged: the run's console lines up to the stop, oldest first, a
     * helper's as `[info] neo.find "the search box" -> e90 …`, your `console.log` lines as written.
     * At most 40 lines and 4,000 characters; when older ones were dropped, the first line says how
     * many. */
    log: string[]
    /** What to do now, in one line. */
    next: string
  }

  interface Neo {
    /** Create and pick a task. `name` is an optional label. Return `task.id`. */
    createTask(name?: string): Promise<Task>
    /** Pick a task. An unknown id throws `task_not_found`; recover with
     * `context.pages()` and each page's `taskId`. */
    task(id: string): Promise<Task>
    /** Solves the CAPTCHA on a page. Every verdict, unsolved too, is a value to branch on. */
    solveCaptcha(page: Page): Promise<{
      verdict:
        | 'solved'
        | 'token_applied'
        | 'already_clear'
        | 'unsolved'
        | 'cancelled'
        | 'unknown'
      type: string | null
      reason: string
      message: string
      durationMs: number
      paidTasks: number
      /** The last provider that charged for this solve, including a failed task. Free solves return null. */
      provider: 'capsolver' | '2captcha' | null
    }>
    /**
     * The Jev helpers: `find`, `findAll`, `check`, `choose`, `stop`. Only when this Neo has a Jev key;
     * the `run` description lists them then. Without one `find`, `findAll`, `check` and `choose`
     * throw `jev_unavailable`.
     *
     * Jev, a fast decision model, reads the page at the moment of the call, so a script can act on
     * pages you have not looked at. Write as far ahead as you can see, run it, then write the next
     * stretch from what came back. End a script where the next step depends on something you must
     * read first.
     *
     * A helper that is not sure stops the script instead of guessing. It throws a `NeoError` with
     * `code: 'stopped'` and `stop: Stop`; leave it uncaught. The run then ends normally and returns
     * `{ stopped: Stop }`; `stopped.log` lists what ran and what you logged. A stop is not an error
     * and undoes nothing: the tab is as the last finished step left it. Carry on from that tab by
     * hand (`getByRef` on a lookalike, or `ariaSnapshot()`); do not run the script again from the top.
     *
     * Each call looks at the newest tab `page` opened during this run (a popup), else `page`, and
     * writes one console line (one more when it follows a new tab):
     * `neo.find "the search box" -> e90 searchbox "Search" p1.00`.
     *
     * `find`: the one element `ask` describes in plain words ("the product search box"). Not there
     * yet: looks again every ~300 ms until `timeout` (default 5000 ms; 0 looks once). Stops as
     * `not_found` when nothing fits, as `ambiguous` when two fit; `optional: true` returns null
     * instead. `role` keeps only elements of that ARIA role. The Locator belongs to the tab the call
     * looked at.
     */
    find(
      page: Page,
      ask: string,
      options?: { optional?: false; timeout?: number; role?: string },
    ): Promise<pw.Locator>
    find(
      page: Page,
      ask: string,
      options: { optional: true; timeout?: number; role?: string },
    ): Promise<pw.Locator | null>
    /** Every element `ask` describes, in page order; count and index in code (`rows[8]`), never in the
     * ask. No match is an empty array. Stops as `unsure` when it cannot tell for some element. */
    findAll(
      page: Page,
      ask: string,
      options?: { role?: string },
    ): Promise<pw.Locator[]>
    /** Whether the page shows `claim` is true, read with the page's text. With `timeout` (ms) it looks
     * again every ~300 ms until it is true: a wait for a state, in place of a sleep. Default 0: look
     * once. Stops as `unsure` when it cannot tell at the last look. */
    check(
      page: Page,
      claim: string,
      options?: { timeout?: number },
    ): Promise<boolean>
    /** Which of `options` the page answers `question` with, returned exactly as given. Add `'other'`
     * yourself when none may fit. Stops as `unsure` when no option clearly leads. */
    choose<T extends string>(
      page: Page,
      question: string,
      options: readonly T[],
    ): Promise<T>
    /** Ends the script here with the same hand-back a helper gives, `why` in your own words. For when
     * your own check says the page is not what you expected. Synchronous: no `await`. */
    stop(page: Page, why: string): never
  }

  const neo: Neo
  /** The picked task's last-used open tab, else another open tab, else a new one; without a
   * task, a NEW tab with no task. `neo.task(id)` binds it to an open tab of the task at once,
   * unless this run already holds that tab's Page; otherwise it picks at its first action.
   * Once it has a tab, it keeps it for the run. */
  const page: Page
  /** `pages()` lists the tabs this run may use (when the host binds runs to their task, only
   * this task's tabs). `newPage()` opens in the picked task, or with no task. */
  const context: pw.BrowserContext
  const browser: pw.Browser
  const expect: Expect
  const playwright: {
    selectors: { setTestIdAttribute(name: string): void }
    errors: { TimeoutError: typeof TimeoutError }
  }
  /** Node's Buffer: `screenshot` and `pdf` resolve to one; `setInputFiles` takes one. */
  var Buffer: BufferConstructor

  /** Every SDK failure. */
  class NeoError extends Error {
    constructor(message: string)
    /** e.g. `task_not_found`, `task_page_missing`, `page_closed`, `method_unavailable`,
     * `stopped`. */
    code?: string
    /** Optional detail, e.g. `target_crashed` beside primary `internal_error`. Ignore unknown values. */
    diagnosticCode?: string
    /** Who must fix it. */
    owner?: 'neo' | 'script' | 'website' | 'host'
    /** `unknown`: a write may have happened. Look at the page before repeating it. */
    delivery?: 'not_sent' | 'done' | 'unknown'
    /**
     * A repeat is safe: nothing was sent, or the call only read. Never a promise that a repeat
     * will succeed; read `message` before trying again.
     */
    retryable?: boolean
    operationId?: string
    /** With `code: 'stopped'`: why and where a Jev helper or `neo.stop` ended the script. */
    stop?: Stop
    /**
     * With `code: 'covered'`: what took the pointer at the element's point until the timeout
     * ran out. The message shows it, its controls and what to do; `ref` works with `getByRef`.
     */
    cover?: {
      ref: string
      role: string
      name: string
      kind:
        | 'page'
        | 'frame'
        | 'partial'
        | 'label'
        | 'wrapper'
        | 'labels'
        | 'nothing'
        | 'folded'
      controls: { ref: string; role: string; name: string }[]
    }
  }
  /** A timeout, as Playwright's. */
  class TimeoutError extends NeoError {}
  // Actions and reads wait up to 30 s (expect: 5 s, toPass: no limit); pass { timeout } to fail sooner, 0 for none.

  // Also given to every script: `console` (its lines come back with the result),
  // `setTimeout` and `clearTimeout`. The engine is QuickJS: no `Intl`, no Node APIs but Buffer.
}

/**
 * Where Neo differs from Playwright 1.64. A member Neo lacks fails `method_unavailable` and names
 * what to use instead, and most refusals say how to do it; these are the differences a script
 * meets without such an error, or that change what a later run sees:
 * - Init scripts and bindings (`addInitScript`, `exposeBinding`, `exposeFunction`, and the
 *   `exposeFunctions` option of `addInitScript` and `evaluate`) last until the run ends. A
 *   context's reaches each tab at the run's first write to it, so a popup's first document and
 *   cross-site iframes run without it. A call the page does not await may run after the op that
 *   caused it returned.
 * - `page.emulateMedia` and `page.setViewportSize` stay on the tab in later runs, until it closes.
 * - `waitForFunction` polls every 100 ms for `polling: 'raf'` (background tabs fire no animation
 *   frames); the handle it resolves to holds the value, which `jsonValue()` reads.
 * - A result nested more than about 50 objects deep fails `Maximum call stack size exceeded`.
 * - `goto` opens http, https, data, about and javascript URLs; a `javascript:` URL runs, then
 *   fails `net::ERR_ABORTED`. `reload` opens http, https, data and about. `file:` pages are
 *   refused: write a local page with `page.setContent`, which refuses `waitUntil: 'networkidle'`.
 * - Path reads (`setInputFiles`, `drop`, script and style tags, init scripts) must be inside the
 *   host's `files.readDirs`, the downloads folder by default, so a later run can upload what
 *   `download.path()` returned. A `{ name, mimeType, buffer }` payload needs no folder and totals
 *   under 10 MiB a call. A directory upload needs its tab in front, which the tabs Neo opens for
 *   a script never are.
 * - In hosted BrowserOS a script receives no console messages and no page errors, by design.
 */

/**
 * Walkthrough: Hacker News, top five links in five tabs
 *
 * // Run 1 — create the task and return its id
 * const task = await neo.createTask('hn top 5')
 * return task.id                                   // "hn-top-5-3f9a"
 *
 * // Run 2 — plain Playwright; every tab lands in the task
 * await neo.task('hn-top-5-3f9a')
 * await page.goto('https://news.ycombinator.com')
 * const links = await page.locator('.titleline > a')
 *   .evaluateAll(as => as.slice(0, 5).map(a => a.href))
 * for (const url of links) await (await context.newPage()).goto(url)
 *
 * // Run 3 — pick one tab of the task
 * const task = await neo.task('hn-top-5-3f9a')
 * const page = task.pages().find(p => p.url().includes('github.com'))
 * return await page.title()
 *
 * // Run 4 — done
 * await (await neo.task('hn-top-5-3f9a')).close()
 */
