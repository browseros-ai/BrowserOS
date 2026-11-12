/** Drives the handler pull across the real bootstrap boundary, retaining transport bytes. */
import { loadPrelude, SEED, type WireExchange } from './prelude-runtime'

/** One trigger followed by cleanup covers registration, pull, resolution and removal. */
export async function recordLocatorHandlerWire(
  script: string,
  prelude?: string,
): Promise<{ exchanges: WireExchange[]; result: string }> {
  const exchanges: WireExchange[] = []
  let recording = false
  let first = true
  let finishPull: ((value: { uid: null }) => void) | undefined
  let resolved!: () => void
  let drained!: () => void
  const handlerResolved = new Promise<void>((resolve) => {
    resolved = resolve
  })
  const pullDrained = new Promise<void>((resolve) => {
    drained = resolve
  })
  const runtime = loadPrelude({
    seed: SEED,
    now: 1_700_000_000_000,
    prelude,
    answer: ({ name, input }) => {
      if (!recording) return null
      if (name === 'page.addLocatorHandler') return { uid: 1 }
      if (name === 'page.locatorHandlerTriggered') {
        if (first) {
          first = false
          return { uid: 1 }
        }
        return new Promise<{ uid: null }>((resolve) => {
          finishPull = resolve
        })
      }
      if (name === 'page.resolveLocatorHandler') {
        if (input.remove) finishPull?.({ uid: null })
        resolved()
        return null
      }
      if (name === 'page.removeLocatorHandler') {
        finishPull?.({ uid: null })
        return null
      }
      throw new Error(`Unexpected handler operation: ${name}`)
    },
    onWire: (exchange) => {
      if (!recording) return
      exchanges.push(exchange)
      if (
        exchange.op === 'page.locatorHandlerTriggered' &&
        JSON.parse(exchange.reply).ok?.uid === null
      )
        drained()
    },
  })
  await runtime.ready
  recording = true
  await runtime.run(script)
  // Resolution belongs to the background pull, not the script's add promise. Wait at that
  // boundary before removing the registration; no wall-clock delay orders these exchanges.
  await handlerResolved
  const result = await runtime.run(`
    await page.removeLocatorHandler(page.overlay)
    return page.seen
  `)
  await pullDrained
  return { exchanges, result: JSON.stringify(result) }
}
