import { describe, expect, it, mock } from 'bun:test'
import type { ConversationPanelAssignment } from '@browseros/shared/schemas/conversation-panels'
import type { UIMessage } from 'ai'
import type { ConversationRunState } from './conversation-run-client'
import {
  PanelConversationAttachment,
  type PanelConversationAttachmentDeps,
  type PanelSnapshot,
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
    async (_snapshot: PanelSnapshot, _isCurrent: () => boolean) => {},
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
    let resolve!: (state: ConversationRunState | undefined) => void
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
    let resolve!: (state: ConversationRunState | undefined) => void
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
    let resolveOld!: (state: ConversationRunState | undefined) => void
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
    expect(f.attach.mock.calls[0]?.[0].liveRunId).toBe('r2')
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

describe('opening a conversation from History', () => {
  const question: UIMessage = {
    id: 'u1',
    role: 'user',
    parts: [{ type: 'text', text: 'Open example.com and report its title' }],
  }
  // What the stored row holds while the turn runs: the checkpoint at its tool call.
  const checkpoint: UIMessage = {
    id: 'a1',
    role: 'assistant',
    parts: [{ type: 'step-start' }],
  }
  const answer: UIMessage = {
    id: 'a1',
    role: 'assistant',
    parts: [{ type: 'text', text: 'Example Domain' }],
  }
  const run = (
    status: ConversationRunState['status'],
    messages: UIMessage[],
  ): ConversationRunState => ({
    conversationId: 'chat',
    runId: 'r1',
    status,
    messages,
  })
  const other: ConversationPanelAssignment = {
    tabId: 7,
    conversationId: 'other',
    runId: 'r0',
    status: 'completed',
  }
  const serverRuns = async (id: string): Promise<ConversationRunState> =>
    id === other.conversationId
      ? { conversationId: id, runId: 'r0', status: 'completed', messages: [] }
      : run('running', [question])
  const neverCancelled = () => false
  /** Installs the way the hook does: after detaching, and only while still current. */
  function installs(f: ReturnType<typeof fixture>, gate = Promise.resolve()) {
    const installed: PanelSnapshot[] = []
    f.attach.mockImplementation(async (snapshot, isCurrent) => {
      await gate
      if (isCurrent()) installed.push(snapshot)
    })
    return installed
  }

  it('follows a running run from its first chunk instead of freezing at the checkpoint', async () => {
    const f = fixture()
    await f.controller.open(
      'chat',
      [question, checkpoint],
      run('running', [question]),
      neverCancelled,
    )
    expect(f.attach.mock.calls.map(([snapshot]) => snapshot)).toEqual([
      {
        source: 'opened',
        conversationId: 'chat',
        messages: [question],
        liveRunId: 'r1',
      },
    ])
    f.controller.dispose()
  })

  it('shows a run that finished after the row was read whole, once, without following it', async () => {
    const f = fixture()
    await f.controller.open(
      'chat',
      [question, checkpoint],
      run('completed', [question, answer]),
      neverCancelled,
    )
    await tick()
    expect(f.attach).toHaveBeenCalledTimes(1)
    const [snapshot] = f.attach.mock.calls[0] ?? []
    expect(snapshot?.messages).toEqual([question, answer])
    expect(snapshot?.liveRunId).toBeUndefined()
    f.controller.dispose()
  })

  it('shows the stored transcript when the server knows no run', async () => {
    const f = fixture()
    await f.controller.open(
      'chat',
      [question, answer],
      undefined,
      neverCancelled,
    )
    const [snapshot] = f.attach.mock.calls[0] ?? []
    expect(snapshot?.messages).toEqual([question, answer])
    expect(snapshot?.liveRunId).toBeUndefined()
    f.controller.dispose()
  })

  it('installs nothing for a selection superseded by a newer one before it lands', async () => {
    const f = fixture()
    let release!: () => void
    const installed = installs(
      f,
      new Promise((resolve) => {
        release = resolve
      }),
    )
    const older = f.controller.open(
      'older',
      [question],
      undefined,
      neverCancelled,
    )
    const newer = f.controller.open(
      'chat',
      [question],
      run('running', [question]),
      neverCancelled,
    )
    release()
    await Promise.all([older, newer])
    expect(installed.map((snapshot) => snapshot.conversationId)).toEqual([
      'chat',
    ])
    f.controller.dispose()
  })

  it('installs nothing when its restore is cancelled while the view detaches', async () => {
    const f = fixture(serverRuns)
    let release!: () => void
    const installed = installs(
      f,
      new Promise((resolve) => {
        release = resolve
      }),
    )
    let cancelled = false
    const opening = f.controller.open(
      'chat',
      [question],
      run('running', [question]),
      () => cancelled,
    )
    cancelled = true // a newer History selection is still reading its row
    release()
    await opening
    f.controller.retry()
    await tick()
    expect(installed).toEqual([])
    f.controller.dispose()
  })

  it('gives way to a new run the broker assigns the tab while it lands', async () => {
    const f = fixture()
    let release!: () => void
    const installed = installs(
      f,
      new Promise((resolve) => {
        release = resolve
      }),
    )
    const opening = f.controller.open(
      'older',
      [question],
      undefined,
      neverCancelled,
    )
    f.controller.update(view())
    release()
    await opening
    await tick()
    expect(installed.map((snapshot) => snapshot.source)).toEqual(['assigned'])
    f.controller.dispose()
  })

  it('retries a failed stream against the History selection, not the tab binding it replaced', async () => {
    const f = fixture(serverRuns)
    f.controller.update(other)
    await tick()
    await f.controller.open(
      'chat',
      [question],
      run('running', [question]),
      neverCancelled,
    )
    f.controller.retry() // the SDK reports a resumed-stream error through onError
    await tick()
    const last = f.attach.mock.calls.at(-1)?.[0]
    expect(last?.conversationId).toBe('chat')
    expect(last?.liveRunId).toBe('r1')
    f.controller.dispose()
  })

  it('keeps following a History selection when the tab loses its old binding', async () => {
    const f = fixture(serverRuns)
    f.controller.update(other)
    await tick()
    await f.controller.open(
      'chat',
      [question],
      run('running', [question]),
      neverCancelled,
    )
    f.controller.retry()
    f.controller.update(undefined)
    await tick()
    expect(f.clear).not.toHaveBeenCalled()
    expect(f.attach.mock.calls.at(-1)?.[0].conversationId).toBe('chat')
    expect(f.attach).toHaveBeenCalledTimes(3)
    f.controller.dispose()
  })

  it('stops following a History selection once New conversation retires it', async () => {
    const f = fixture(async () => run('running', [question]))
    await f.controller.open(
      'chat',
      [question],
      run('running', [question]),
      neverCancelled,
    )
    f.controller.retire('chat')
    f.controller.retry()
    await tick()
    expect(f.attach).toHaveBeenCalledTimes(1)
    f.controller.dispose()
  })
})
