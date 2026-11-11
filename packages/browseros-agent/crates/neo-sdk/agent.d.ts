/**
 * agent.d.ts: what a script sent to Neo's MCP `run` tool can use. Give this file to your
 * agent together with Neo's MCP URL.
 *
 * `code` is the body of an async function: `await` each call, `return` a JSON value.
 * Each `run` call starts with fresh JavaScript. Only browser tabs carry over.
 * A task is your own set of tabs. Start one per user request and keep its id.
 *   first run:        const task = await neo.createTask('hn top 5'); return task.id
 *   every later run:  const task = await neo.task('<that id>')
 *   when finished:    await task.close()
 * Using the built-in `page` without a task creates a new task for it; the reply
 * tells you that task's id. In a script that fetched a task, the built-in `page` is
 * refused: take a tab from the task instead (`const page = task.page(104)` is allowed).
 * Tab ids here are short for reading; real ones are large numbers, e.g. 1157427185.
 *
 * Every reply, a failed one too, ends with one line per task the script touched:
 *   [task hn-top-5-3f9a · tabs: 101 https://news.ycombinator.com/, 102 https://…]
 *   [new task t-7c2e was created for this script's built-in page · tabs: 107 https://…]
 * Lost an id? `(await neo.tabs()).find(p => p.url().includes('ycombinator'))?.taskId`
 * Tasks live in Neo's memory: they end with `task.close()` or a Neo restart.
 */

// A hand-out, not application code: no workspace tsconfig includes it, so these globals
// never leak into the apps. It type-checks on its own against Playwright's types.
/// <reference types="node" />
import type { Expect } from '@playwright/test'
import type * as pw from 'playwright-core'

declare module 'playwright-core' {
  /** Playwright's Page, plus: */
  interface Page {
    /** The tab's id. It stays the same across runs. `null` only for the built-in `page`
     * before its tab opens. */
    readonly tabId: number | null
    /** The task that owns this tab; `null` for the user's own tabs. */
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

  interface Neo {
    /** Start a task. `name` is an optional label people see. Keep `task.id`. */
    createTask(name?: string): Promise<Task>
    /** Get a task you created in an earlier run. An unknown id throws `task_not_found`. */
    task(id: string): Promise<Task>
    /** Every open tab, with the task it belongs to. Use it if you lost a task id. */
    tabs(): Promise<Page[]>
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
    }>
  }

  const neo: Neo
  /** Without a task: a new tab in a new task (the reply names it). After `neo.task()` or
   * `neo.createTask()`: refused until you declare your own `const page`. */
  const page: Page
  /** `pages()` lists every tab the run knows. In a script that fetched a task,
   * `newPage()` is refused: use `task.newPage()`. */
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
    /** e.g. `task_not_found`, `task_page_required`, `page_closed`, `method_unavailable`. */
    code?: string
    /** Who must fix it. */
    owner?: 'neo' | 'script' | 'website' | 'host'
    /** `unknown`: a write may have happened. Look at the page before repeating it. */
    delivery?: 'not_sent' | 'done' | 'unknown'
    retryable?: boolean
    operationId?: string
  }
  /** A timeout, as Playwright's. */
  class TimeoutError extends NeoError {}

  // Also given to every script: `console` (its lines come back with the result),
  // `setTimeout` and `clearTimeout`. The engine is QuickJS: no `Intl`, no Node APIs but Buffer.
}

/**
 * Walkthrough: "open Hacker News and the top five links in five tabs". Four `run` calls.
 *
 * // Run 1 — create the task
 * const task = await neo.createTask('hn top 5')
 * return task.id                                   // "hn-top-5-3f9a"
 *
 * // Run 2 — open HN, then the five links, all in this task
 * const task = await neo.task('hn-top-5-3f9a')
 * const hn = await task.newPage()
 * await hn.goto('https://news.ycombinator.com')
 * const links = await hn.locator('.titleline > a')
 *   .evaluateAll(as => as.slice(0, 5).map(a => a.href))
 * for (const url of links) {
 *   const tab = await task.newPage()
 *   await tab.goto(url)
 * }
 * return task.pages().map(p => ({ tabId: p.tabId, url: p.url() }))   // six tabs
 *
 * // Run 3 — later, work in one of them
 * const task = await neo.task('hn-top-5-3f9a')
 * const page = task.page(104)      // or task.pages().find(p => p.url().includes('github.com'))
 * return await page.title()
 *
 * // Run 4 — done
 * const task = await neo.task('hn-top-5-3f9a')
 * await task.close()                                // closes all six tabs
 */
