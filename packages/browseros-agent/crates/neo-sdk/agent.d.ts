/**
 * agent.d.ts: what a script sent to Neo's MCP `run` tool can use. Give this file to your
 * agent together with Neo's MCP URL.
 *
 * `code` is the body of an async function: `await` each call, `return` a JSON value.
 * Each `run` call starts with fresh JavaScript; browser tabs carry over.
 * With no task, built-in `page` and `context.newPage()` each open a NEW tab with no task.
 * To keep tabs across runs, create a task and return its id:
 *   first run:        const task = await neo.createTask('hn top 5'); return task.id
 *   every later run:  const task = await neo.task('<that id>')
 *   when finished:    await task.close()
 * After `neo.createTask()` or `neo.task()`, built-in `page` uses that task's last-used
 * open tab, else another open tab, else a new one. `context.newPage()` opens in the task.
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
 * Any script may use any tab or task; `context.pages()` lists every tab.
 * Tasks live in Neo's memory: they end with `task.close()` or a Neo restart.
 */

// A hand-out, not application code: no workspace tsconfig includes it, so these globals
// never leak into the apps. It type-checks on its own against Playwright's types.
/// <reference types="node" />
import type { Expect } from '@playwright/test'
import type * as pw from 'playwright-core'

declare module 'playwright-core' {
  interface Locator {
    /** Neo-only. Strictly resolves one attached element and returns its node-bound ref.
     * Reuses an engine ref or mints n<k>, prefixed f<seq> in other documents. Keep the ref
     * with its task's page for later runs. Removal or navigation makes it stale; minting
     * changes neither snapshots nor actionability checks. */
    ref(options?: { timeout?: number }): Promise<string>
  }
  /** Playwright's Page, plus: */
  interface Page {
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
    }>
  }

  const neo: Neo
  /** The picked task's last-used open tab, else another open tab, else a new one; without a
   * task, a NEW tab with no task. `neo.task(id)` binds it to an open tab of the task at once,
   * unless this run already holds that tab's Page; otherwise it picks at its first action.
   * Once it has a tab, it keeps it for the run. */
  const page: Page
  /** `pages()` lists every tab. `newPage()` opens in the picked task, or with no task. */
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
    /** e.g. `task_not_found`, `task_page_missing`, `page_closed`, `method_unavailable`. */
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
  // Actions and reads wait up to 30 s (expect: 5 s); pass { timeout } to fail sooner, 0 for none.

  // Also given to every script: `console` (its lines come back with the result),
  // `setTimeout` and `clearTimeout`. The engine is QuickJS: no `Intl`, no Node APIs but Buffer.
}

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
