import { describe, expect, it, mock } from 'bun:test'
import type { ConversationPanelAssignment } from '@browseros/shared/schemas/conversation-panels'
import type { ConversationRunState } from './conversation-run-client'
import {
  PanelConversationAttachment,
  type PanelConversationAttachmentDeps,
} from './panel-conversation-attachment'

const view = (runId = 'r1'): ConversationPanelAssignment => ({
  tabId: 7,
  conversationId: 'chat',
  runId,
  status: 'running',
})
const state = (runId = 'r1'): ConversationRunState => ({
  conversationId: 'chat',
  runId,
  status: 'running',
  messages: [],
})
const tick = () => new Promise((resolve) => setTimeout(resolve, 5))
function fixture(
  load: PanelConversationAttachmentDeps['load'] = async () => state(),
) {
  const attach = mock(
    async (_state: ConversationRunState, _isCurrent: () => boolean) => {},
  )
  const clear = mock((_id: string) => {})
  const ownsLocalStream = mock((_id: string, _runId: string) => false)
  const reportError = mock((_error: unknown) => {})
  const controller = new PanelConversationAttachment({
    load,
    attach,
    clear,
    ownsLocalStream,
    reportError,
    retryMs: 1,
  })
  return { controller, attach, clear, ownsLocalStream, reportError }
}

describe('panel attachment lifecycle', () => {
  it('discards pending hydration when New conversation retires the old one', async () => {
    let resolve!: (state: ConversationRunState) => void
    const f = fixture(
      () =>
        new Promise((r) => {
          resolve = r
        }),
    )
    f.controller.update(view())
    f.controller.retire('chat')
    resolve(state())
    await tick()
    f.controller.update(view()) // delayed heartbeat from before deletion
    expect(f.attach).not.toHaveBeenCalled()
    f.controller.dispose()
  })

  it('invalidates a pending attachment when its tab binding disappears', async () => {
    let resolve!: (state: ConversationRunState) => void
    const f = fixture(
      () =>
        new Promise((r) => {
          resolve = r
        }),
    )
    f.controller.update(view())
    f.controller.update(undefined)
    resolve(state())
    await tick()
    expect(f.clear).toHaveBeenCalledWith('chat')
    expect(f.attach).not.toHaveBeenCalled()
    f.controller.dispose()
  })

  it('keeps a completed conversation attached and does not hydrate on heartbeats', async () => {
    const f = fixture()
    f.controller.update(view())
    await tick()
    f.controller.update(view())
    f.controller.update({ ...view(), status: 'completed' })
    expect(f.attach).toHaveBeenCalledTimes(1)
    expect(f.clear).not.toHaveBeenCalled()
    f.controller.dispose()
  })

  it('retries fetch failures and SDK errors without waiting for a storage change', async () => {
    let attempts = 0
    const f = fixture(async () => {
      if (++attempts === 1) throw new Error('offline')
      return state()
    })
    f.controller.update(view())
    await tick()
    expect(f.attach).toHaveBeenCalledTimes(1)
    f.controller.retry() // SDK reports onError without rejecting resumeStream
    await tick()
    expect(f.attach).toHaveBeenCalledTimes(2)
    f.controller.dispose()
  })

  it('rejects old hydration after a newer run assignment arrives', async () => {
    let resolveOld!: (state: ConversationRunState) => void
    let calls = 0
    const f = fixture(async () => {
      if (++calls === 1)
        return new Promise((r) => {
          resolveOld = r
        })
      return state('r2')
    })
    f.controller.update(view())
    f.controller.update(view('r2'))
    await tick()
    resolveOld(state())
    await tick()
    expect(f.attach).toHaveBeenCalledTimes(1)
    expect(f.attach.mock.calls[0]?.[0].runId).toBe('r2')
    f.controller.dispose()
  })

  it('adopts a locally submitted run without interrupting its POST stream', async () => {
    const f = fixture()
    f.ownsLocalStream.mockImplementation(() => true)
    f.controller.update(view())
    await tick()
    expect(f.attach).not.toHaveBeenCalled()
    f.controller.dispose()
  })
})
