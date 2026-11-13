/** Record calls at the bootstrap boundary, retaining JSON spelling and key order. */
import {
  type Answer,
  loadPrelude,
  SEED,
  type WireExchange,
} from './prelude-runtime'

/** Startup frame reads are outside the script; every later engine exchange is retained. */
export async function recordWireBytes(
  script: string,
  answer: Answer,
  prelude?: string,
): Promise<{ exchanges: WireExchange[]; result: string }> {
  const exchanges: WireExchange[] = []
  let recording = false
  const runtime = loadPrelude({
    seed: SEED,
    now: 1_700_000_000_000,
    prelude,
    answer: (call) => (recording ? answer(call) : null),
    onWire: (exchange) => {
      if (recording) exchanges.push(exchange)
    },
  })
  await runtime.ready
  recording = true
  const value = await runtime.run(script)
  return { exchanges, result: JSON.stringify(value ?? null) }
}
