import { jsonObject, stringField } from './json'

/** Root CDP is restricted to creating and removing the one externally owned borrowed target. */
export interface BorrowedTargets {
  create(url: string): Promise<string>
  close(targetId: string): Promise<void>
}

async function rootCommand(options: {
  cdpPort: number
  method: string
  params: object
}) {
  const response = await fetch(
    `http://127.0.0.1:${options.cdpPort}/json/version`,
    { signal: AbortSignal.timeout(10_000) },
  )
  if (!response.ok)
    throw new Error(`Root CDP discovery failed: ${response.status}`)
  const version = jsonObject(await response.json(), 'Root CDP version')
  const endpoint = stringField(version, 'webSocketDebuggerUrl')
  return await new Promise<ReturnType<typeof jsonObject>>((resolve, reject) => {
    const socket = new WebSocket(endpoint)
    let finished = false
    const timeout = setTimeout(() => {
      finish(new Error(`Root CDP ${options.method} timed out`))
    }, 15_000)
    function finish(error?: Error, result?: ReturnType<typeof jsonObject>) {
      if (finished) return
      finished = true
      clearTimeout(timeout)
      socket.close()
      if (error) reject(error)
      else if (result) resolve(result)
      else reject(new Error(`Root CDP ${options.method} returned no result`))
    }
    socket.addEventListener('open', () => {
      socket.send(
        JSON.stringify({
          id: 1,
          method: options.method,
          params: options.params,
        }),
      )
    })
    socket.addEventListener('message', (event) => {
      try {
        if (typeof event.data !== 'string')
          throw new Error('Root CDP reply was not text')
        const reply = jsonObject(JSON.parse(event.data), 'Root CDP reply')
        if (reply.id !== 1) return
        if (reply.error) {
          finish(
            new Error(
              `Root CDP ${options.method}: ${JSON.stringify(reply.error)}`,
            ),
          )
          return
        }
        finish(undefined, jsonObject(reply.result, 'Root CDP result'))
      } catch (error) {
        finish(error instanceof Error ? error : new Error(String(error)))
      }
    })
    socket.addEventListener('error', () =>
      finish(new Error(`Root CDP ${options.method} socket failed`)),
    )
    socket.addEventListener('close', () =>
      finish(new Error(`Root CDP ${options.method} socket closed`)),
    )
  })
}

export function borrowedTargets(cdpPort: number): BorrowedTargets {
  return {
    async create(url) {
      const result = await rootCommand({
        cdpPort,
        method: 'Target.createTarget',
        params: { url },
      })
      return stringField(result, 'targetId')
    },
    async close(targetId) {
      const result = await rootCommand({
        cdpPort,
        method: 'Target.getTargets',
        params: {},
      })
      if (!Array.isArray(result.targetInfos))
        throw new Error('Root CDP omitted targetInfos')
      // The body closes the borrowed page in finally. A missing target is successful cleanup;
      // a surviving one is closed only by the ID this harness created, never browser-wide.
      const exists = result.targetInfos.some((value) => {
        const target = jsonObject(value, 'Root CDP target')
        return stringField(target, 'targetId') === targetId
      })
      if (!exists) return
      const closed = await rootCommand({
        cdpPort,
        method: 'Target.closeTarget',
        params: { targetId },
      })
      if (closed.success !== true)
        throw new Error('Root CDP did not confirm borrowed target cleanup')
    },
  }
}
