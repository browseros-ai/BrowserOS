/**
 * Interception setup must precede the activation that opens a picker. Listener APIs return
 * synchronously; this run-local barrier keeps later browser calls behind their native setup.
 * Event operations bypass it so setup and the event pump cannot wait on themselves.
 */
import { OPS, type OpName } from './ops.generated'

const pending = new Set<Promise<void>>()

export function trackChooserSetup(setup: Promise<void>): void {
  pending.add(setup)
  const remove = () => {
    pending.delete(setup)
  }
  void setup.then(remove, remove)
}

export function readyChoosers(op: OpName): Promise<unknown> | null {
  // A lazy page's frames read must be able to create its tab before its lease is armed.
  // Pure observations cannot activate a picker; only subsequent writes need the barrier.
  if (OPS[op].effect === 'read') return null
  return pending.size ? Promise.all([...pending]) : null
}
