/** Neo: Loads the production SDK verbatim into the Node worker and binds its native door. */
import fs from 'node:fs'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

export async function openSdk(cdpPort, options, { onUncaught } = {}) {
  const require = createRequire(import.meta.url)
  const addon = require(process.env.NEO_NODE_ADDON || './addon/neo_node.node')
  const runtime = await addon.connect(cdpPort)
  let native
  try {
    native = await runtime.openRun(options)
    let finishing = false
    const parked = new Promise(() => {})
    const uncaught =
      onUncaught ||
      ((error) =>
        setImmediate(() => {
          throw error
        }))
    const body = fs.readFileSync(
      new URL('../../neo-quickjs-injected/dist/sdk.js', import.meta.url),
      'utf8',
    )
    const door = {
      get: (request) => native.get(request),
      callWithEvents: async (message) =>
        JSON.parse(await native.callWithEvents(message)),
      abort: (id, reason) => native.abort(id, reason),
      subscribe: (request) => native.subscribe(request),
      drain: (until) => native.drain(until),
      nextEvents: () =>
        finishing
          ? parked
          : native.nextEvents().catch((error) => {
              if (!finishing) uncaught(error)
              return parked
            }),
      log: (line) => native.log(line),
    }
    // Direct eval preserves the classic script's completion value, comments and trailing semicolon.
    const { createRun } = vm.compileFunction(
      `return eval(${JSON.stringify(body)})`,
      ['__neo_runtime'],
      {
        filename: fileURLToPath(
          new URL('../../neo-quickjs-injected/dist/sdk.js', import.meta.url),
        ),
      },
    )(door)
    const sdk = createRun(JSON.parse(native.init))
    // Playwright Test owns Node's uncaught-error handler and attributes this to the active test.
    sdk.uncaught.catch(uncaught)
    return {
      ...sdk,
      native,
      runtime,
      async finish() {
        finishing = true
        try {
          return await native.finish()
        } finally {
          await runtime.close()
        }
      },
    }
  } catch (error) {
    try {
      await native?.finish()
    } finally {
      await runtime.close()
    }
    throw error
  }
}
