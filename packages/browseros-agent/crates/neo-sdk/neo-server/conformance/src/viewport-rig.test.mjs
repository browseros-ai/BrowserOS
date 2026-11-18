import { expect, test } from 'bun:test'
import { sizePages } from './viewport-rig.mjs'

function fakeCdp() {
  let socket
  const commands = []
  const popupSizing = Promise.withResolvers()
  const send = (message) => socket.send(JSON.stringify(message))
  const attach = (sessionId, type, waitingForDebugger) =>
    send({
      method: 'Target.attachedToTarget',
      params: {
        sessionId,
        targetInfo: { targetId: sessionId, type, url: 'about:blank' },
        waitingForDebugger,
      },
    })
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch(request, server) {
      if (new URL(request.url).pathname === '/json/version')
        return Response.json({
          webSocketDebuggerUrl: `ws://${server.hostname}:${server.port}/devtools/browser/test`,
        })
      if (!server.upgrade(request)) return new Response(null, { status: 400 })
    },
    websocket: {
      open(ws) {
        socket = ws
      },
      message(_ws, data) {
        const command = JSON.parse(data)
        commands.push(command)
        if (command.method === 'Target.setAutoAttach') {
          attach('initial', 'page', false)
          attach('worker', 'service_worker', true)
        }
        if (
          command.sessionId === 'popup' &&
          command.method === 'Emulation.setDeviceMetricsOverride'
        ) {
          popupSizing.resolve(() => send({ id: command.id, result: {} }))
          return
        }
        const fail =
          command.sessionId === 'rejected' &&
          command.method === 'Emulation.setDeviceMetricsOverride'
        send({
          id: command.id,
          ...(fail
            ? { error: { message: 'cannot size page' } }
            : { result: {} }),
        })
      },
    },
  })
  return {
    server,
    commands,
    attach,
    popupSizing: popupSizing.promise,
    close: () => socket.close(),
  }
}

test('sizes initial and paused new pages before release, and releases other targets', async () => {
  const { server, commands, attach, popupSizing } = fakeCdp()
  let rig
  try {
    rig = await sizePages({ cdpHttp: server.url.origin })
    attach('popup', 'page', true)
    const acknowledgeSizing = await popupSizing
    expect(
      commands.filter((command) => command.sessionId === 'popup'),
    ).toHaveLength(1)
    acknowledgeSizing()
    await rig.settle()
    expect(commands[0]).toMatchObject({
      method: 'Target.setAutoAttach',
      params: { autoAttach: true, waitForDebuggerOnStart: true, flatten: true },
    })
    for (const sessionId of ['initial', 'popup']) {
      const calls = commands.filter(
        (command) => command.sessionId === sessionId,
      )
      expect(calls.map((command) => command.method)).toEqual([
        'Emulation.setDeviceMetricsOverride',
        'Runtime.runIfWaitingForDebugger',
      ])
      expect(calls[0].params).toEqual({
        width: 1280,
        height: 720,
        screenWidth: 1280,
        screenHeight: 720,
        deviceScaleFactor: 1,
        mobile: false,
        screenOrientation: { angle: 0, type: 'landscapePrimary' },
      })
    }
    expect(
      commands
        .filter((command) => command.sessionId === 'worker')
        .map((command) => command.method),
    ).toEqual(['Runtime.runIfWaitingForDebugger'])
    expect(rig.pages()).toEqual([
      expect.objectContaining({
        targetId: 'initial',
        sizedMs: expect.any(Number),
        releasedMs: expect.any(Number),
      }),
      expect.objectContaining({
        targetId: 'popup',
        sizedMs: expect.any(Number),
        releasedMs: expect.any(Number),
      }),
    ])
    expect(rig.pages().every((page) => !page.error)).toBe(true)
  } finally {
    await rig?.stop()
    await server.stop(true)
  }
})

test('records late sizing and rejected overrides and detects a lost socket', async () => {
  const { server, attach, close } = fakeCdp()
  let rig
  try {
    rig = await sizePages({ cdpHttp: server.url.origin })
    attach('unpaused', 'page', false)
    attach('rejected', 'page', true)
    await rig.settle()
    expect(rig.pages().slice(1)).toEqual([
      expect.objectContaining({
        targetId: 'unpaused',
        error: 'new page was not paused',
        releasedMs: expect.any(Number),
      }),
      expect.objectContaining({
        targetId: 'rejected',
        error: 'size: cannot size page',
        releasedMs: expect.any(Number),
      }),
    ])
    expect(rig.health()).toBe('ok')
    close()
    await rig.settle()
    expect(rig.health()).toBe('lost')
  } finally {
    await rig?.stop()
    await server.stop(true)
  }
})
