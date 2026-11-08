/**
 * Playwright: `client/clientHelper.ts` `evaluationScript`
 * Playwright: `client/disposable.ts` `DisposableObject`
 *
 * What `page.addInitScript` and `browserContext.addInitScript` send, Playwright's
 * `evaluationScript`, and the `Disposable` they answer, which removes the script again.
 */

import type { ApiName, Connection } from './connection'
import { isTargetClosedError } from './errors'

/** The init script commands, on a page, the context, or a script's `disposable@` guid. */
export type InitScriptCommands = {
  addInitScript: {
    params: { source: string }
    result: { disposable: { guid: string } }
  }
  dispose: { params: Record<string, never>; result: Record<string, never> }
}

/** Playwright's `addInitScript` script: a function (called with `arg`), source text, or
 * `{ content }` (or `{ path }`, which Neo refuses). */
export type InitScript =
  | ((arg: never) => unknown)
  | string
  | { path?: string; content?: string }

/** Playwright: `client/clientHelper.ts` `evaluationScript`: `const argString = Object.is(arg, undefined) ? 'undefined' : JSON.stringify(arg);` */
export function evaluationScript(
  apiName: ApiName,
  fun: InitScript,
  arg?: unknown,
): string {
  if (typeof fun === 'function') {
    const source = fun.toString()
    const argString = Object.is(arg, undefined)
      ? 'undefined'
      : JSON.stringify(arg)
    return `(${source})(${argString})`
  }
  if (arg !== undefined)
    throw new Error('Cannot evaluate a string with arguments')
  if (typeof fun === 'string') return fun
  if (fun.content !== undefined) return fun.content
  if (fun.path !== undefined)
    // Deviation: `client/clientHelper.ts` `evaluationScript`: `let source = await fs.promises.readFile(fun.path, 'utf8');` Neo reads files only from the run's artifact directory.
    throw new Error(
      `${apiName}: path: this run has no artifact directory, so Neo cannot read "${fun.path}"`,
    )
  throw new Error('Either path or content property must be present')
}

/** Playwright: `client/disposable.ts` `DisposableObject.dispose`: `if (isTargetClosedError(e))` */
export class Disposable {
  readonly #connection: Connection
  readonly #guid: string

  constructor(connection: Connection, guid: string) {
    this.#connection = connection
    this.#guid = guid
  }

  /** Removes the init script: no later document runs it. A second dispose is done. */
  async dispose(): Promise<void> {
    try {
      await this.#connection.sendInit(
        'disposable.dispose',
        this.#guid,
        'dispose',
        {},
        0,
      )
    } catch (error) {
      if (isTargetClosedError(error)) return
      throw error
    }
  }
}
