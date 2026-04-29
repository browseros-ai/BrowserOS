/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock } from 'bun:test'

describe('createTerminalSocketEvents', () => {
  afterEach(() => {
    mock.restore()
  })

  it('resolves limactl only when a terminal socket opens', async () => {
    const close = mock(() => {})
    const send = mock(() => {})
    const session = {
      close: mock(() => {}),
      resize: mock(() => {}),
      writeInput: mock(() => {}),
    }
    const createTerminalSession = mock(() => session)
    const actualTerminalSession = await import(
      '../../../src/api/services/terminal/terminal-session'
    )

    mock.module('../../../src/api/services/terminal/terminal-session', () => ({
      ...actualTerminalSession,
      createTerminalSession,
    }))

    const { createTerminalSocketEvents } = await import(
      '../../../src/api/routes/terminal'
    )
    const resolveLimactlPath = mock(() => '/tmp/fake-limactl')

    const events = createTerminalSocketEvents({
      containerName: 'gateway',
      limaHome: '/tmp/lima',
      limactlPath: resolveLimactlPath,
      vmName: 'browseros-vm',
    })

    expect(resolveLimactlPath).not.toHaveBeenCalled()

    events.onOpen(new Event('open'), { send, close })

    expect(resolveLimactlPath).toHaveBeenCalledTimes(1)
    expect(createTerminalSession).toHaveBeenCalledWith(
      expect.objectContaining({
        containerName: 'gateway',
        limaHome: '/tmp/lima',
        limactlPath: '/tmp/fake-limactl',
        vmName: 'browseros-vm',
        workingDir: actualTerminalSession.TERMINAL_HOME_DIR,
      }),
    )
    expect(close).not.toHaveBeenCalled()
  })

  it('sends an error and closes when the limactl resolver throws', async () => {
    const close = mock(() => {})
    const send = mock(() => {})
    const createTerminalSession = mock(() => {
      throw new Error('should not start a session')
    })
    const actualTerminalSession = await import(
      '../../../src/api/services/terminal/terminal-session'
    )

    mock.module('../../../src/api/services/terminal/terminal-session', () => ({
      ...actualTerminalSession,
      createTerminalSession,
    }))

    const { createTerminalSocketEvents } = await import(
      '../../../src/api/routes/terminal'
    )
    const events = createTerminalSocketEvents({
      containerName: 'gateway',
      limaHome: '/tmp/lima',
      limactlPath: () => {
        throw new Error('limactl not found')
      },
      vmName: 'browseros-vm',
    })

    events.onOpen(new Event('open'), { send, close })

    expect(createTerminalSession).not.toHaveBeenCalled()
    expect(send).toHaveBeenCalledWith(
      JSON.stringify({ type: 'error', message: 'limactl not found' }),
    )
    expect(close).toHaveBeenCalledTimes(1)
  })
})
