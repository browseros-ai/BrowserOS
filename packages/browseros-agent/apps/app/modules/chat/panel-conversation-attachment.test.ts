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
  loadStored: PanelConversationAttachmentDeps['loadStored'] = async () =>
    undefined,
) {
  const attach = mock(
    async (_snapshot: PanelSnapshot, _isCurrent: () => boolean) => {},
  )
  const clear = mock((_id: string) => {})
  const ownsLocalStream = mock((_id: string, _runId: string) => false)
  const reportError = mock((_error: unknown) => {})
  const storedReads = mock(loadStored)
  const controller = new PanelConversationAttachment({
    load,
    loadStored: storedReads,
    attach,
    clear,
    ownsLocalStream,
    reportError,
    retryMs: 1,
  })
  return {
    controller,
    attach,
    clear,
    ownsLocalStream,
    reportError,
    storedReads,
  }
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

describe('after this panel stops its run', () => {
  const question: UIMessage = {
    id: 'u1',
    role: 'user',
    parts: [{ type: 'text', text: 'Start the sixty-second browser script.' }],
  }
  // What the server seals once the run unwinds. The panel that pressed Stop was detached before
  // this arrived on its stream, so it still holds the same tool part as `input-available`.
  const stopped: UIMessage = {
    id: 'a1',
    role: 'assistant',
    parts: [
      { type: 'step-start' },
      {
        type: 'tool-run',
        toolCallId: 't1',
        state: 'output-error',
        input: { code: 'await new Promise(() => {})' },
        errorText: 'Stopped',
      },
    ],
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
  const neverCancelled = () => false
  const stopAnswered = Promise.resolve()
  /**
   * The panel as the hook drives it: an install detaches first, lands only while still current,
   * and moves the panel to the snapshot's conversation, which is what `isShown` reads.
   */
  function panel(f: ReturnType<typeof fixture>, gate = Promise.resolve()) {
    const screen = { shows: 'chat', installed: [] as PanelSnapshot[] }
    f.attach.mockImplementation(async (snapshot, isCurrent) => {
      await gate
      if (!isCurrent()) return
      screen.shows = snapshot.conversationId
      screen.installed.push(snapshot)
    })
    return { screen, isShown: () => screen.shows === 'chat' }
  }

  it('shows the stopped tool part in the panel that started the turn', async () => {
    const f = fixture(async () => run('aborted', [question, stopped]))
    const { screen, isShown } = panel(f)
    f.ownsLocalStream.mockImplementation(() => true)
    f.controller.beginLocalTurn('chat')
    f.controller.update(view()) // the broker assigns the tab the run its POST started
    await f.controller.reload('chat', stopAnswered, isShown)
    expect(screen.installed).toHaveLength(1)
    expect(screen.installed[0]?.messages).toEqual([question, stopped])
    expect(screen.installed[0]?.liveRunId).toBeUndefined()
    // An `assigned` install cancels a History restore still loading; a reload must not.
    expect(screen.installed[0]?.source).not.toBe('assigned')
    f.controller.dispose()
  })

  it('shows the stopped tool part in a panel that opened the run from History', async () => {
    const f = fixture(async () => run('aborted', [question, stopped]))
    const { screen, isShown } = panel(f)
    await f.controller.open(
      'chat',
      [question],
      run('running', [question]),
      neverCancelled,
    )
    await f.controller.reload('chat', stopAnswered, isShown)
    expect(
      screen.installed.map(({ messages, liveRunId }) => ({
        messages,
        liveRunId,
      })),
    ).toEqual([
      { messages: [question], liveRunId: 'r1' },
      { messages: [question, stopped], liveRunId: undefined },
    ])
    f.controller.dispose()
  })

  it('replays a run that is still unwinding to its end', async () => {
    // Stop answers before the run has unwound, so the state can still say `running`.
    const f = fixture(async () => run('running', [question]))
    const { screen, isShown } = panel(f)
    await f.controller.reload('chat', stopAnswered, isShown)
    expect(screen.installed[0]?.messages).toEqual([question])
    expect(screen.installed[0]?.liveRunId).toBe('r1')
    f.controller.dispose()
  })

  it('shows the stored conversation once the server no longer retains the run', async () => {
    const f = fixture(
      async () => undefined,
      async (id) => (id === 'chat' ? [question, stopped] : undefined),
    )
    const { screen, isShown } = panel(f)
    await f.controller.reload('chat', stopAnswered, isShown)
    expect(screen.installed[0]?.messages).toEqual([question, stopped])
    expect(screen.installed[0]?.liveRunId).toBeUndefined()
    f.controller.dispose()
  })

  it('keeps what the panel shows when the turn was never stored', async () => {
    // An incognito turn: no run retained and no stored conversation to fall back to.
    const f = fixture(async () => undefined)
    const { screen, isShown } = panel(f)
    await f.controller.reload('chat', stopAnswered, isShown)
    expect(f.storedReads).toHaveBeenCalledWith('chat')
    expect(screen.installed).toEqual([])
    f.controller.dispose()
  })

  it('changes nothing once the panel has opened another conversation from History', async () => {
    let respond!: (state: ConversationRunState) => void
    const f = fixture(
      () =>
        new Promise((resolve) => {
          respond = resolve
        }),
    )
    const { screen, isShown } = panel(f)
    const reloading = f.controller.reload('chat', stopAnswered, isShown)
    await tick()
    await f.controller.open('other', [question], undefined, neverCancelled)
    respond(run('aborted', [question, stopped]))
    await reloading
    expect(screen.installed.map((s) => s.conversationId)).toEqual(['other'])
    f.controller.dispose()
  })

  it('changes nothing once New conversation has replaced the stopped one', async () => {
    let respond!: (state: ConversationRunState) => void
    const f = fixture(
      () =>
        new Promise((resolve) => {
          respond = resolve
        }),
    )
    const { screen, isShown } = panel(f)
    const reloading = f.controller.reload('chat', stopAnswered, isShown)
    await tick()
    f.controller.retire('chat')
    screen.shows = 'fresh'
    respond(run('aborted', [question, stopped]))
    await reloading
    expect(screen.installed).toEqual([])
    f.controller.dispose()
  })

  it('leaves a History selection that lands while Stop is answered in place', async () => {
    const f = fixture(async () => run('aborted', [question, stopped]))
    let release!: () => void
    const { screen, isShown } = panel(
      f,
      new Promise((resolve) => {
        release = resolve
      }),
    )
    const opening = f.controller.open(
      'other',
      [question],
      undefined,
      neverCancelled,
    )
    const reloading = f.controller.reload('chat', stopAnswered, isShown)
    release()
    await Promise.all([opening, reloading])
    expect(screen.installed.map((s) => s.conversationId)).toEqual(['other'])
    f.controller.dispose()
  })

  it('gives way to a turn this panel sends before Stop is answered', async () => {
    const f = fixture(async () => run('aborted', [question, stopped]))
    const { screen, isShown } = panel(f)
    let answer!: () => void
    const reloading = f.controller.reload(
      'chat',
      new Promise<void>((resolve) => {
        answer = resolve
      }),
      isShown,
    )
    f.controller.beginLocalTurn('chat')
    answer()
    await reloading
    // Installing would detach the new turn's stream and replace its messages.
    expect(screen.installed).toEqual([])
    f.controller.dispose()
  })

  it('reports a state it cannot read and installs nothing', async () => {
    const f = fixture(async () => {
      throw new Error('offline')
    })
    const { screen, isShown } = panel(f)
    await f.controller.reload('chat', stopAnswered, isShown)
    expect(f.reportError).toHaveBeenCalledTimes(1)
    expect(screen.installed).toEqual([])
    f.controller.dispose()
  })
})
