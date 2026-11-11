/**
 * Give this file to your agent with Neo's MCP URL. `run` takes an async function body:
 * await SDK calls and return a JSON value. Every run is fresh JavaScript; no variables
 * survive. Only browser tabs carry over. Use one task per user request:
 *
 *   First: const task = await neo.createTask('HN top 5'); return { taskId: task.id }
 *   Later: const task = await neo.task('hn-top-5-3f9a')
 *   Done:  const task = await neo.task('hn-top-5-3f9a'); await task.close()
 *
 * Keep the returned id. Replies that ran a script, including failures, end with a
 * line per created or fetched task, so a throw cannot lose the id:
 *   [task hn-top-5-3f9a · tabs: 101 https://news.ycombinator.com/]
 * Empty tasks say `tabs: none`; closed tasks say `closed · its tabs were closed`.
 *
 * Without a host task, built-in `page` opens a blank tab in a new unnamed task on first
 * use, every run. Its reply says `[new task t-7c2e was created for this script's built-in
 * page · tabs: 101 about:blank]`. After `neo.createTask` or `neo.task`, built-in `page`
 * is refused if it has no tab yet; an existing tab keeps working. Your own `const page`
 * is allowed: pick `task.page(tabId)`, `task.pages()[0]`, or `await task.newPage()`.
 * A host's taskId keeps its last-used tab as the default, or opens one on first use.
 * Host tasks appear in reply lines only when the script fetches them with `neo.task`.
 *
 * Tasks live in memory until closed or Neo restarts. Agent tabs idle for 30 minutes
 * may close. Find surviving tabs with `neo.tabs()` and read each page's `taskId`.
 */

// Standalone handout, outside workspace tsconfigs so SDK globals do not enter application code.
/// <reference types="node" />
import type { Expect } from '@playwright/test'
import type * as pw from 'playwright-core'

// Pages returned by Playwright methods carry the same ids as task pages.
declare module 'playwright-core' {
  interface Page {
    /** Stable across runs; null only for built-in `page` before its tab opens. */
    readonly tabId: number | null
    /** The owning task, or null for a tab without one, including the user's tabs. */
    readonly taskId: string | null
  }
}

declare global {
  /** One piece of work, such as one user request. It owns the tabs it opens. */
  interface Task {
    /** Neo makes the id. Keep it for `neo.task(id)` in later runs. */
    readonly id: string
    /** This task's known open tabs, oldest first, including popups. Synchronous. */
    pages(): Page[]
    /** Throws `task_page_missing`, listing this task's tab ids, if the tab is absent. */
    page(tabId: number): Page
    /** Opens a background tab owned by this task. */
    newPage(): Promise<Page>
    /** Closes owned tabs and forgets the task; on refusal, throws and keeps tabs left. */
    close(): Promise<void>
  }

  /** Playwright 1.64.1's Page plus Neo's ids. See `run` for supported methods and differences. */
  type Page = pw.Page

  interface Neo {
    /** Starts a task. The optional name is trimmed and limited to 200 characters. */
    createTask(name?: string): Promise<Task>
    /** Fetches a task from an earlier run. An unknown id throws `task_not_found`. */
    task(id: string): Promise<Task>
    /** Every open tab, including the user's tabs and other tasks' tabs. */
    tabs(): Promise<Page[]>
    /** Await before other actions on this page. Every verdict, unsolved too, is a value. */
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
  const page: Page
  /**
   * `pages()` lists every tab the run knows. `newPage()` joins the host or auto task,
   * but is refused after `neo.createTask` or `neo.task`; use `task.newPage()` then.
   */
  const context: pw.BrowserContext
  const browser: pw.Browser
  const expect: Expect
  const playwright: {
    selectors: { setTestIdAttribute(name: string): void }
    errors: { TimeoutError: typeof TimeoutError }
  }
  /** Screenshots and PDFs return Buffer; file upload payloads accept it. */
  var Buffer: BufferConstructor

  /** SDK failures carry these fields; a script-created error has only its message. */
  class NeoError extends Error {
    constructor(message: string)
    code?: string
    /** Who must fix the failure. */
    owner?: 'neo' | 'script' | 'website' | 'host'
    /** `unknown` means a write may have happened. Inspect the page before retrying. */
    delivery?: 'not_sent' | 'done' | 'unknown'
    retryable?: boolean
    operationId?: string
  }
  class TimeoutError extends NeoError {}

  // Also supplied: console (log/info/warn/error/debug lines return with the result),
  // setTimeout and clearTimeout (numeric timer ids, valid only within this run).
  // The engine is QuickJS, with no Intl. No Node APIs other than Buffer are supplied.
}

/**
 * Hacker News: the top five links in five tabs. Each block is a separate `run`.
 *
 * // Run 1: start. Keep the returned task id and tab id.
 * const task = await neo.createTask('HN top 5')
 * const hn = await task.newPage()
 * await hn.goto('https://news.ycombinator.com')
 * return { taskId: task.id, tabId: hn.tabId }
 * // Example result: { "taskId": "hn-top-5-3f9a", "tabId": 101 }
 *
 * // Run 2: fetch that task and open the five links.
 * const task = await neo.task('hn-top-5-3f9a')
 * const page = task.page(101)
 * const urls = await page.evaluate(() =>
 *   [...document.querySelectorAll('a')]
 *     .filter(a => a.matches('.titleline > a')).slice(0, 5).map(a => a.href))
 * for (const url of urls) await (await task.newPage()).goto(url)
 * return { taskId: task.id, tabs: task.pages().map(p => [p.tabId, p.url()]) }
 * // Six tabs now belong to this task, including the Hacker News tab.
 *
 * // Run 3: pick a tab id from that result, for example 103.
 * const task = await neo.task('hn-top-5-3f9a')
 * const page = task.page(103)
 * return await page.locator('body').innerText()
 *
 * // Run 4: finish. Close only this task's tabs.
 * const task = await neo.task('hn-top-5-3f9a')
 * await task.close()
 * // Reply: [task hn-top-5-3f9a closed · its tabs were closed]
 */
