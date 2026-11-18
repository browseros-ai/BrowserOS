/**
 * Interception setup must precede the activation that opens a picker. Listener APIs return
 * synchronously; this run-local barrier keeps a later write behind the native setup of its own
 * tab, so `page.on('filechooser', fn)` then `await page.click(...)` catches the chooser that
 * click opens. Event operations bypass it so setup and the event pump cannot wait on themselves.
 *
 * The barrier is per tab. A chooser opens only on the page whose input was activated, so another
 * tab's setup never holds a write. Waiting on every tab's setups would let one tab whose native
 * interception is slow, bounded only by CDP's 60 s request timeout, stall every write on every
 * tab of the run.
 */
import type { TabId } from './generated'
import { OPS, type OpName } from './generated/ops'

/**
 * The page a setup belongs to, read when a write checks it: the run's unopened page has no tab
 * until its first op binds one. Structural, so this module needs no import of `Page`.
 */
type SetupPage = { readonly _tab: TabId | null }

const pending = new Set<{ page: SetupPage; done: Promise<void> }>()

export function trackChooserSetup(page: SetupPage, setup: Promise<void>): void {
  const entry = { page, done: setup }
  pending.add(entry)
  const remove = () => {
    pending.delete(entry)
  }
  void setup.then(remove, remove)
}

/**
 * What `op`, about to send `input`, must wait for. A write on tab T waits for T's setups and for
 * those of a page with no tab yet, which may turn out to be T. A write on the unopened page waits
 * for every setup, since the door picks its tab only after this. A write that names no page
 * (`context.newPage`, `addInitScript`, `exposeBinding`, `dispose`, `download.saveAs`,
 * `page.saveScreenshot`) dispatches no input to any page, so it waits for none.
 */
export function readyChoosers(
  op: OpName,
  input: object,
): Promise<unknown> | null {
  // A lazy page's frames read must be able to create its tab before its lease is armed.
  // Pure observations cannot activate a picker; only subsequent writes need the barrier.
  if (OPS[op].effect === 'read') return null
  const brand = pageBrand(input)
  if (brand == null) return null
  const tab =
    typeof brand === 'object' && 'tab' in brand && typeof brand.tab === 'number'
      ? brand.tab
      : null
  const held = [...pending]
    .filter(
      ({ page }) => tab === null || page._tab === null || page._tab === tab,
    )
    .map(({ done }) => done)
  return held.length ? Promise.all(held) : null
}

/**
 * The brand an op acts on, found where the door's `page_brand` finds it
 * (neo-server/src/builtins/playwright_ops.rs): a non-null `page`, else `target.page`.
 */
function pageBrand(input: object): unknown {
  if ('page' in input && input.page != null) return input.page
  if (!('target' in input)) return undefined
  const { target } = input
  return typeof target === 'object' && target !== null && 'page' in target
    ? target.page
    : undefined
}
