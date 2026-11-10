/** Neo: The fixture and independent observer agree on the end-of-test boundary before
 * reference CDP disconnect resets Playwright's emulation. These exclusive per-phase files
 * carry lifecycle acknowledgements only; browser operations still use the actual engines. */
import fs from 'node:fs'
import path from 'node:path'

const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const files = (directory) => ({
  request: path.join(directory, 'observation-request.json'),
  reply: path.join(directory, 'observation-reply.json'),
})

/** The fixture retains its connection until the supervisor has read final effects. Native
 * callers invoke this after run completion/drain; reference callers invoke it before the
 * host-only disconnect. A missing supervisor cannot leave a fixture parked indefinitely. */
export async function requestObservation(
  directory,
  { timeoutMs = 20_000 } = {},
) {
  const { request, reply } = files(directory)
  fs.writeFileSync(
    request,
    JSON.stringify({ stage: 'before-engine-disconnect' }),
    {
      flag: 'wx',
    },
  )
  const deadline = Date.now() + timeoutMs
  while (!fs.existsSync(reply)) {
    if (Date.now() >= deadline)
      throw new Error('Final observation acknowledgement timed out')
    await pause(10)
  }
  const response = JSON.parse(fs.readFileSync(reply, 'utf8'))
  if (response.status !== 'captured')
    throw new Error(`Final observation failed: ${response.error}`)
}

/** The supervisor starts polling before launching the runner, and stops/reaps this task
 * on every runner exit. Snapshot loss stays an infrastructure error, never a post-disconnect
 * substitute. A negative acknowledgement lets fixture teardown still close its connection. */
export function observationFence(directory, capture, signal) {
  const { request, reply } = files(directory)
  const state = { observation: undefined, error: undefined, requested: false }
  let stopped = false
  const pending = (async () => {
    while (!stopped && !signal?.aborted) {
      if (!fs.existsSync(request)) {
        await pause(10)
        continue
      }
      state.requested = true
      let timeout
      try {
        state.observation = await Promise.race([
          capture(),
          new Promise((_, reject) => {
            timeout = setTimeout(
              () =>
                reject(
                  new Error('Final observation exceeded its finite guard'),
                ),
              15_000,
            )
          }),
        ])
        fs.writeFileSync(
          path.join(directory, 'final-observation.json'),
          JSON.stringify(state.observation),
          { flag: 'wx' },
        )
      } catch (error) {
        state.error = String(error.stack || error)
      } finally {
        clearTimeout(timeout)
        const temporary = `${reply}.tmp`
        fs.writeFileSync(
          temporary,
          JSON.stringify(
            state.error
              ? { status: 'failed', error: state.error }
              : { status: 'captured' },
          ),
          { flag: 'wx' },
        )
        fs.renameSync(temporary, reply)
      }
      return
    }
  })().catch((error) => {
    state.error = String(error.stack || error)
  })
  return {
    state,
    async stop() {
      stopped = true
      await pending
    },
  }
}
