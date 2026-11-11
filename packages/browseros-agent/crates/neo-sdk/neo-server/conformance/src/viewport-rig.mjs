/** Owns a browser-level CDP connection that sets Playwright's default viewport before a
 * new page starts. The override remains attached for the run, including between test workers. */
export const viewport = { width: 1280, height: 720, deviceScaleFactor: 1 }
const timeout = 5000

export async function sizePages({ cdpHttp }) {
  const version = await fetch(`${cdpHttp}/json/version`, {
    signal: AbortSignal.timeout(timeout),
  })
  if (!version.ok) throw new Error(`CDP discovery returned ${version.status}`)
  const { webSocketDebuggerUrl } = await version.json()
  const socket = new WebSocket(webSocketDebuggerUrl)
  const pages = [],
    attaching = new Set(),
    pending = new Map()
  let nextId = 0,
    starting = true,
    lost = false

  function disconnect(error) {
    lost = true
    for (const { reject } of pending.values()) reject(error)
  }
  function send(method, params = {}, sessionId) {
    if (lost) throw new Error('CDP connection lost')
    const id = ++nextId
    let timer
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject })
      timer = setTimeout(
        () => reject(new Error(`${method} timed out`)),
        timeout,
      )
      socket.send(JSON.stringify({ id, method, params, sessionId }))
    }).finally(() => {
      clearTimeout(timer)
      pending.delete(id)
    })
  }

  async function attach({ sessionId, targetInfo, waitingForDebugger }) {
    const page = targetInfo.type === 'page'
    const entry = {
      targetId: targetInfo.targetId,
      url: targetInfo.url,
      waitingForDebugger,
      attachedAt: Date.now(),
    }
    if (page) pages.push(entry)
    const errors = []
    try {
      if (page) {
        if (!starting && !waitingForDebugger)
          errors.push('new page was not paused')
        await send(
          'Emulation.setDeviceMetricsOverride',
          {
            ...viewport,
            screenWidth: viewport.width,
            screenHeight: viewport.height,
            mobile: false,
            screenOrientation: { angle: 0, type: 'landscapePrimary' },
          },
          sessionId,
        )
        entry.sizedMs = Date.now() - entry.attachedAt
      }
    } catch (error) {
      errors.push(`size: ${error.message}`)
    }
    // Auto-attach pauses other target types too. Release every target, even after an override
    // failure, so the harness cannot leave a renderer or service worker waiting on the debugger.
    try {
      await send('Runtime.runIfWaitingForDebugger', {}, sessionId)
      entry.releasedMs = Date.now() - entry.attachedAt
    } catch (error) {
      errors.push(`release: ${error.message}`)
      if (!page) disconnect(error)
    }
    if (errors.length) entry.error = errors.join('; ')
  }
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data)
    if (message.method === 'Target.attachedToTarget') {
      const task = attach(message.params)
      attaching.add(task)
      void task.finally(() => attaching.delete(task))
    } else {
      const command = pending.get(message.id)
      if (message.error) command?.reject(new Error(message.error.message))
      else command?.resolve(message.result)
    }
  })
  socket.addEventListener('close', () =>
    disconnect(new Error('CDP connection closed')),
  )
  socket.addEventListener('error', () =>
    disconnect(new Error('CDP connection failed')),
  )

  // A browser-level round trip puts queued attach events before the evidence check. Drain
  // their commands too, since the release reply can arrive after the test has already returned.
  async function settle() {
    try {
      await send('Target.getTargetInfo')
    } catch (error) {
      disconnect(error)
    }
    while (attaching.size) await Promise.all(attaching)
  }
  async function stop() {
    socket.close()
    disconnect(new Error('viewport rig stopped'))
    while (attaching.size) await Promise.all(attaching)
  }
  try {
    await new Promise((resolve, reject) => {
      const signal = AbortSignal.timeout(timeout)
      socket.addEventListener('open', resolve, { once: true, signal })
      socket.addEventListener('error', reject, { once: true, signal })
      socket.addEventListener('close', reject, { once: true, signal })
      signal.addEventListener('abort', () => reject(signal.reason), {
        once: true,
      })
    })
    await send('Target.setAutoAttach', {
      autoAttach: true,
      waitForDebuggerOnStart: true,
      flatten: true,
    })
    await settle()
    starting = false
    return {
      pages: () => pages.map((entry) => ({ ...entry })),
      health: () => (lost ? 'lost' : 'ok'),
      settle,
      stop,
    }
  } catch (error) {
    await stop()
    throw new Error(`viewport rig startup: ${error.message || error.type}`)
  }
}
