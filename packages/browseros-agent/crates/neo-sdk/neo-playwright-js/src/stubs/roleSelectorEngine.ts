/**
 * Build-time stand-in for upstream roleSelectorEngine.ts.
 * InjectedScript registers `'role'` and `'internal:role'` in its constructor, so
 * `createRoleEngine` must succeed without throwing; invoking `queryAll` throws because
 * role queries execute in the Rust APC engine, not in the page.
 */
import type { SelectorEngine, SelectorRoot } from './selectorEngine'

export function createRoleEngine(internal: boolean): SelectorEngine {
  const engineName = internal ? 'internal:role' : 'role'
  return {
    queryAll(_root: SelectorRoot, _selector: string): Element[] {
      throw new Error(
        `The '${engineName}' selector engine runs in Rust, not in the page`,
      )
    },
  }
}
