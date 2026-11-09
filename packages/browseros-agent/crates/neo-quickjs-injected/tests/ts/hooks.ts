/** Neo: compile a hook author's file, including returns that must be rejected. */
/// <reference path="../../hooks.d.ts" />
import type {
  BrowserContext,
  ElementHandle,
  Frame,
  Locator,
  Page,
} from 'playwright-core'
import type { HookFilter, HookGroup, HookKind } from '../../hooks'

neo.on('beforeRun', async (event, ctx) => {
  const page: Page = event.page
  const context: BrowserContext = event.context
  ctx.state.startUrl = page.url()
  ctx.trace('checking script', event.script.length, context.pages().length)
  const answer = await ctx.host('policy.canRun', { meta: ctx.meta })
  if (ctx.signal.aborted || answer !== true)
    return { block: 'Owner approval required' }
  return { allow: true }
})

neo.on('afterRun', (event, ctx) => {
  if (event.status !== 'ok') {
    ctx.trace(event.error?.name, ctx.state.startUrl)
    return { note: 'The run did not finish successfully' }
  }
  return {
    replace: { approved: true, value: event.value },
    note: 'Policy checked',
  }
})

neo.on(
  'beforeAction',
  { kinds: ['fill'], groups: ['type'] },
  async (event, ctx) => {
    const kind: HookKind | null = event.kind
    const group: HookGroup | null = event.group
    const frame: Frame | undefined = event.frame
    const locator: Locator | undefined = event.locator
    ctx.trace(kind, group, frame?.url(), event.location?.line, event.method)
    if (locator && !(await locator.isEditable()))
      return { block: 'Field is not editable' }
    return { replaceArgs: { value: 'approved text' } }
  },
)
neo.on('afterAction', { groups: ['read'] }, (event) => ({
  replace: event.result,
}))
neo.on('beforeRun', () => undefined)
neo.on('afterRun', undefined, () => ({ replace: undefined }))
neo.on('beforeAction', null, () => ({ allow: true }))
neo.on('afterAction', () => undefined)

// @ts-expect-error beforeRun cannot replace the script or its arguments.
neo.on('beforeRun', () => ({ replaceArgs: { script: 'return 1' } }))
// @ts-expect-error an after hook cannot block.
neo.on('afterRun', () => ({ block: 'too late' }))
// @ts-expect-error an empty decision is not keep; return undefined.
neo.on('afterAction', () => ({}))
// @ts-expect-error an unknown kind must not silently disable a policy.
neo.on('beforeAction', { kinds: ['flll'] }, () => undefined)
// @ts-expect-error groups use the generated literal vocabulary too.
neo.on('afterAction', { groups: ['typing'] }, () => undefined)
// @ts-expect-error filters only apply to action events.
neo.on('beforeRun', { groups: ['type'] }, () => undefined)
// @ts-expect-error block and allow are mutually exclusive.
neo.on('beforeAction', () => ({ allow: true, block: 'no' }))
// @ts-expect-error before decisions cannot be hidden alongside an after decision.
neo.on('afterRun', () => ({ replace: 1, allow: true }))
// @ts-expect-error after decisions cannot be hidden alongside a before decision.
neo.on('beforeRun', () => ({ allow: true, note: 'not allowed here' }))
// @ts-expect-error an own note key must contain a string, even alongside replace.
neo.on('afterRun', () => ({ replace: 1, note: undefined }))
// @ts-expect-error async handlers must resolve to the event's allowed decision.
neo.on('beforeRun', async () => ({ replace: 'not a before decision' }))
/** Neo: an invalid filter fixture, rejected even when assigned outside a registration. */
// @ts-expect-error a filter must supply at least one list.
const emptyFilter: HookFilter = {}
void emptyFilter

neo.on('beforeRun', (_event, ctx) => {
  // @ts-expect-error metadata cannot be replaced.
  ctx.meta = null
  // @ts-expect-error host args must be JSON, not executable code.
  void ctx.host('policy.canRun', () => true)
})
neo.on('beforeAction', (event) => {
  const element: ElementHandle | undefined = event.element
  void element?.getAttribute('type')
})

neo.on('afterAction', (event) => {
  const message: string | undefined = event.error?.message
  void message
  const decision: import('../../hooks').AfterActionDecision = {
    // @ts-expect-error afterAction cannot replace arguments.
    replaceArgs: {},
  }
  void decision
})
