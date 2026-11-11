/**
 * `exposeBinding` and `exposeFunction`, on a page and on the context: the page calls a function
 * of the agent's script and gets its result back. Upstream's design, carried over a run:
 *
 *   page: window.foo(1)  the stub Rust installed ─ BindingsController.callBinding
 *     ─ __playwright__binding__(JSON) ─ Runtime.bindingCalled ─ Rust's event log
 *   realm: the pump (events.ts) gets a `binding` record ─ answerBindingCall: the callback,
 *     found by name on its page, then on the context ─ bindingCall.resolve { context, arg }
 *     ─ Rust runs deliverBindingResult in the calling context ─ the page's promise settles
 *
 * Nothing calls into the realm from Rust: a call reaches the script because the pump pulls it.
 * A binding lasts as long as the run (Rust removes it when the run ends) or until its disposable
 * is disposed. The callbacks live here, in the realm, which is why a binding cannot outlive the run.
 */
import { invoke } from './bridge'
import type { BrowserContext } from './context'
import { clientError } from './errors'
import { fromInPageArgument, inPageArgument } from './evaluation'
import type { Frame } from './frame'
import { type Disposable, registration } from './init-script'
import type { Page } from './page'
import type { BindingRecord } from './wire'

/** upstream's `structs.BindingSource`: where the call came from. */
export type BindingSource = {
  context: BrowserContext
  page: Page
  frame: Frame
}

export type BindingCallback = (
  source: BindingSource,
  ...args: unknown[]
) => unknown

/** What a stub posts: upstream's `BindingPayload`. */
type Payload = { name: unknown; seq: unknown; serializedArgs: unknown }

/**
 * upstream's `Page.exposeBinding`: the server's duplicate checks, then the binding, set before
 * Rust installs its stub so a call the page makes at once finds it. `noGlobal` keeps it off
 * `window`, for `addInitScript`'s `exposeFunctions` callbacks.
 */
export async function exposeOnPage(
  page: Page,
  api: string,
  name: string,
  callback: BindingCallback,
  noGlobal = false,
): Promise<Disposable> {
  if (page._exposed.has(name))
    throw clientError(api, `Function "${name}" has been already registered`)
  if (page._context._exposed.has(name))
    throw clientError(
      api,
      `Function "${name}" has been already registered in the browser context`,
    )
  page._exposed.set(name, callback)
  const events = page._context._events
  events.bindingAdded()
  const forget = once(() => {
    page._exposed.delete(name)
    events.bindingRemoved()
  })
  try {
    const { id } = await invoke(
      'page.exposeBinding',
      { api, timeoutMs: 0 },
      {
        page: await page._brand(api),
        name,
        ...(noGlobal ? { noGlobal: true as const } : {}),
      },
    )
    return registration(id, forget)
  } catch (error) {
    forget()
    throw error
  }
}

/** upstream's `BrowserContext.exposeBinding`, as [`exposeOnPage`] for every page of the run. */
export async function exposeOnContext(
  context: BrowserContext,
  api: string,
  name: string,
  callback: BindingCallback,
  noGlobal = false,
): Promise<Disposable> {
  if (context._exposed.has(name))
    throw clientError(api, `Function "${name}" has been already registered`)
  if (context.pages().some((page) => page._exposed.has(name)))
    throw clientError(
      api,
      `Function "${name}" has been already registered in one of the pages`,
    )
  context._exposed.set(name, callback)
  context._events.bindingAdded()
  const forget = once(() => {
    context._exposed.delete(name)
    context._events.bindingRemoved()
  })
  try {
    const { id } = await invoke(
      'context.exposeBinding',
      { api, timeoutMs: 0 },
      { name, ...(noGlobal ? { noGlobal: true as const } : {}) },
    )
    return registration(id, forget)
  } catch (error) {
    forget()
    throw error
  }
}

/**
 * `forget` runs once: a disposable disposed again must not remove the same name exposed anew
 * since, even with the same callback.
 */
function once(forget: () => void): () => void {
  let done = false
  return () => {
    if (done) return
    done = true
    forget()
  }
}

/**
 * Answers one call, as upstream's server (`PageBinding.dispatch`) and client (`BindingCall.call`)
 * do together: the arguments parsed from the page's format, the callback awaited, and its result,
 * or what it threw, handed back to the page. A name this run does not know is another run's
 * binding on the same tab, or one disposed while the call was on its way; it is left for its
 * owner. The answer's own failure (the page went away, the owner took the tab) has nobody to tell.
 */
export async function answerBindingCall(
  page: Page,
  record: BindingRecord,
): Promise<void> {
  let payload: Payload
  try {
    payload = JSON.parse(record.payload) as Payload
  } catch {
    return
  }
  const { name, seq } = payload
  if (typeof name !== 'string') return
  const callback = page._exposed.get(name) ?? page._context._exposed.get(name)
  if (!callback) return
  let answer: Record<string, unknown>
  try {
    if (!Array.isArray(payload.serializedArgs))
      throw new Error(
        'serializedArgs is not an array. This can happen when Array.prototype.toJSON is defined incorrectly',
      )
    const args = payload.serializedArgs.map(fromInPageArgument)
    const frame = await page._bindingFrame(record.frame)
    const result = await callback(
      { context: page._context, page, frame },
      ...args,
    )
    answer = { name, seq, result }
  } catch (error) {
    answer = { name, seq, error }
  }
  let arg: string
  try {
    arg = inPageArgument(answer)
  } catch (error) {
    arg = inPageArgument({ name, seq, error })
  }
  const api = 'bindingCall.resolve'
  await invoke(
    'bindingCall.resolve',
    { api, timeoutMs: 0 },
    { page: await page._brand(api), context: record.context, arg },
  ).catch(() => {})
}
