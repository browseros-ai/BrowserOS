/**
 * What a line of the v1 text inherits of an `aria-hidden` node above it in its document. Upstream
 * marks only the node that carries the attribute, and its nesting shows what the mark hides.
 * `getByRole` reaches those elements only with `includeHidden`, which an agent asks for only where
 * a line, or a line it is nested under, says `[aria-hidden]`. The v1 text moves lines out of that
 * nesting three ways, and the mark must go with each line:
 *
 *   flatten   a wrapper prints no line; a control's or a heading's content prints beside it
 *   cut       the cap prints a kept block after its cut at the left edge, outside its sections
 *   on top    a whole-view layer's content prints first, outside every line around it
 *
 * `'no'`: no `aria-hidden` node is above the line in its document. `'enclosed'`: a line around it
 * says `[aria-hidden]`, so the nesting shows it while the line stays inside. `'carried'`: an
 * `aria-hidden` node above printed no line around it, so the line says it itself. A child
 * document is never hidden by its iframe's ancestors, as `aria-hidden` stops at the document
 * (`isElementHiddenForAria` walks one). Rust's twin is neo-snapshot's `neo/hidden.rs`.
 */
import type { AriaNodeJSON } from './vendored'

export type Hidden = 'no' | 'enclosed' | 'carried'

/**
 * What `node`'s content inherits when it prints with no line of `node` around it, beside
 * `node`'s line or lifted out of it.
 */
export function beside(hidden: Hidden, node: AriaNodeJSON): Hidden {
  if (node.role === 'iframe') return 'no'
  return hidden === 'carried' || node.ariaHidden ? 'carried' : hidden
}

/** What `node`'s content inherits nested under `node`'s line, which says any mark it has. */
export function under(hidden: Hidden, node: AriaNodeJSON): Hidden {
  if (node.role === 'iframe') return 'no'
  return hidden !== 'no' || node.ariaHidden ? 'enclosed' : 'no'
}

/** What a kept block's line inherits: the cap may print it outside the sections around it. */
export function movable(hidden: Hidden): Hidden {
  return hidden === 'no' ? 'no' : 'carried'
}
