import { describe, expect, it } from 'bun:test'
import { createVoiceLoopStore } from './voice-loop.store'

const blob = () => new Blob([new ArrayBuffer(8)], { type: 'audio/wav' })

describe('voice loop store', () => {
  it('starts in idle with empty audio levels and no error', () => {
    const store = createVoiceLoopStore()
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('idle')
    expect(ctx.errorMessage).toBeNull()
    expect(ctx.isWarmingUp).toBe(false)
    expect(ctx.audioLevels).toEqual([0, 0, 0, 0, 0])
    expect(ctx.origin).toBe('normal')
    expect(ctx.chatStreamEndedWhilePending).toBe(false)
  })

  it('OPEN sets isWarmingUp true; WARM_UP_DONE clears it', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    expect(store.getSnapshot().context.isWarmingUp).toBe(true)
    store.send({ type: 'WARM_UP_DONE' })
    expect(store.getSnapshot().context.isWarmingUp).toBe(false)
  })

  it('OPEN moves idle -> listening', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    expect(store.getSnapshot().context.state).toBe('listening')
  })

  it('SPEECH_START in listening moves to capturing', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    expect(store.getSnapshot().context.state).toBe('capturing')
  })

  it('SPEECH_END in capturing moves to transcribing and emits runTranscribe', () => {
    const store = createVoiceLoopStore()
    const emits: Array<{ blob: Blob }> = []
    store.on('runTranscribe', (e) => {
      emits.push(e)
    })
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    const b = blob()
    store.send({ type: 'SPEECH_END', blob: b })
    expect(store.getSnapshot().context.state).toBe('transcribing')
    expect(emits).toHaveLength(1)
    expect(emits[0].blob).toBe(b)
  })

  it('TRANSCRIBE_OK in transcribing moves to responding and emits sendChatMessage', () => {
    const store = createVoiceLoopStore()
    const sent: Array<{ text: string }> = []
    store.on('sendChatMessage', (e) => {
      sent.push(e)
    })
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'hello' })
    expect(store.getSnapshot().context.state).toBe('responding')
    expect(sent.map((e) => e.text)).toEqual(['hello'])
  })

  it('TRANSCRIBE_FAIL in transcribing moves to error with message', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_FAIL', message: 'timeout' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('error')
    expect(ctx.errorMessage).toBe('timeout')
  })

  it('CHAT_STREAMING_ENDED in responding returns to listening', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'hello' })
    store.send({ type: 'CHAT_STREAMING_ENDED' })
    expect(store.getSnapshot().context.state).toBe('listening')
  })

  it('SPEECH_START in responding is a no-op (barge-in uses BARGE_IN_TENTATIVE)', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'hello' })
    store.send({ type: 'SPEECH_START' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('responding')
    expect(ctx.origin).toBe('normal')
  })

  it('BARGE_IN_TENTATIVE from responding moves to barge_in_pending without cancelling', () => {
    const store = createVoiceLoopStore()
    let cancelled = false
    store.on('cancelChatStream', () => {
      cancelled = true
    })
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('barge_in_pending')
    expect(ctx.origin).toBe('barge_in_pending')
    expect(cancelled).toBe(false)
  })

  it('BARGE_IN_TENTATIVE is a no-op when not responding', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    expect(store.getSnapshot().context.state).toBe('listening')
  })

  it('barge-in confirmed path: TRANSCRIBE_OK cancels + interrupts + sends in order', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    const order: string[] = []
    store.on('cancelChatStream', () => order.push('cancel'))
    store.on('markLastAssistantInterrupted', () => order.push('mark'))
    store.on('sendChatMessage', () => order.push('send'))
    store.send({ type: 'SPEECH_END', blob: blob() })
    // SPEECH_END from barge_in_pending starts transcribing but does
    // not cancel yet. Only TRANSCRIBE_OK confirms the interrupt.
    expect(store.getSnapshot().context.state).toBe('transcribing')
    store.send({ type: 'TRANSCRIBE_OK', text: 'wait stop' })
    expect(order).toEqual(['cancel', 'mark', 'send'])
    expect(store.getSnapshot().context.state).toBe('responding')
  })

  it('barge-in safety: TRANSCRIBE_DROPPED returns to responding without cancelling', () => {
    const store = createVoiceLoopStore()
    let cancelled = false
    store.on('cancelChatStream', () => {
      cancelled = true
    })
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_DROPPED', reason: 'hallucination_only' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('responding')
    expect(ctx.origin).toBe('normal')
    expect(cancelled).toBe(false)
  })

  it('barge-in safety: TRANSCRIBE_FAIL during pending does not surface an error', () => {
    const store = createVoiceLoopStore()
    let cancelled = false
    store.on('cancelChatStream', () => {
      cancelled = true
    })
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_FAIL', message: 'network' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('responding')
    expect(ctx.errorMessage).toBeNull()
    expect(cancelled).toBe(false)
  })

  it('TRANSCRIBE_DROPPED from a normal turn returns to listening', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_DROPPED', reason: 'empty' })
    expect(store.getSnapshot().context.state).toBe('listening')
  })

  it('CHAT_STREAMING_ENDED during barge_in_pending stashes the flag', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    store.send({ type: 'CHAT_STREAMING_ENDED' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('barge_in_pending')
    expect(ctx.chatStreamEndedWhilePending).toBe(true)
  })

  it('CHAT_STREAMING_ENDED during transcribing+barge_in_pending stashes the flag', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'CHAT_STREAMING_ENDED' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('transcribing')
    expect(ctx.chatStreamEndedWhilePending).toBe(true)
  })

  it('TRANSCRIBE_DROPPED after chat ended during pending unwinds to listening', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'CHAT_STREAMING_ENDED' })
    store.send({ type: 'TRANSCRIBE_DROPPED', reason: 'hallucination_only' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('listening')
    expect(ctx.chatStreamEndedWhilePending).toBe(false)
  })

  it('SPEECH_ABORTED from capturing returns to listening', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    expect(store.getSnapshot().context.state).toBe('capturing')
    store.send({ type: 'SPEECH_ABORTED' })
    expect(store.getSnapshot().context.state).toBe('listening')
  })

  it('SPEECH_ABORTED from barge_in_pending returns to responding', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    expect(store.getSnapshot().context.state).toBe('barge_in_pending')
    store.send({ type: 'SPEECH_ABORTED' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('responding')
    expect(ctx.origin).toBe('normal')
  })

  it('SPEECH_ABORTED from barge_in_pending after chat ended unwinds to listening', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    store.send({ type: 'CHAT_STREAMING_ENDED' })
    store.send({ type: 'SPEECH_ABORTED' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('listening')
    expect(ctx.chatStreamEndedWhilePending).toBe(false)
  })

  it('SPEECH_ABORTED from other states is a no-op', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_ABORTED' })
    expect(store.getSnapshot().context.state).toBe('listening')
  })

  it('TRANSCRIBE_FAIL after chat ended during pending unwinds to listening', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'BARGE_IN_TENTATIVE' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'CHAT_STREAMING_ENDED' })
    store.send({ type: 'TRANSCRIBE_FAIL', message: 'net' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('listening')
    expect(ctx.errorMessage).toBeNull()
    expect(ctx.chatStreamEndedWhilePending).toBe(false)
  })

  it('STOP_AGENT in responding cancels and returns to listening', () => {
    const store = createVoiceLoopStore()
    const order: string[] = []
    store.on('cancelChatStream', () => order.push('cancel'))
    store.on('markLastAssistantInterrupted', () => order.push('mark'))
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'SPEECH_END', blob: blob() })
    store.send({ type: 'TRANSCRIBE_OK', text: 'first' })
    store.send({ type: 'STOP_AGENT' })
    expect(order).toEqual(['cancel', 'mark'])
    expect(store.getSnapshot().context.state).toBe('listening')
  })

  it('CLOSE from any state goes to closed and emits releaseCapture', () => {
    const store = createVoiceLoopStore()
    let released = false
    store.on('releaseCapture', () => {
      released = true
    })
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'CLOSE' })
    expect(store.getSnapshot().context.state).toBe('closed')
    expect(released).toBe(true)
  })

  it('OPEN from error state clears the error and returns to listening', () => {
    // retry() in useVoiceLoop calls open(), which on success emits
    // OPEN. The error state must therefore yield cleanly to OPEN.
    const store = createVoiceLoopStore()
    store.send({ type: 'ERROR', message: 'mic denied' })
    expect(store.getSnapshot().context.errorMessage).toBe('mic denied')
    store.send({ type: 'OPEN' })
    const ctx = store.getSnapshot().context
    expect(ctx.state).toBe('listening')
    expect(ctx.errorMessage).toBeNull()
  })

  it('AUDIO_LEVELS updates audio levels in any state', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'AUDIO_LEVELS', levels: [10, 20, 30, 40, 50] })
    expect(store.getSnapshot().context.audioLevels).toEqual([
      10, 20, 30, 40, 50,
    ])
    store.send({ type: 'OPEN' })
    store.send({ type: 'SPEECH_START' })
    store.send({ type: 'AUDIO_LEVELS', levels: [1, 2, 3, 4, 5] })
    expect(store.getSnapshot().context.audioLevels).toEqual([1, 2, 3, 4, 5])
  })

  it('no-op: SPEECH_END from listening does not change state', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    let emitCount = 0
    store.on('runTranscribe', () => {
      emitCount++
    })
    store.send({ type: 'SPEECH_END', blob: blob() })
    expect(store.getSnapshot().context.state).toBe('listening')
    expect(emitCount).toBe(0)
  })

  it('no-op: TRANSCRIBE_OK from idle does not change state or emit', () => {
    const store = createVoiceLoopStore()
    let emitCount = 0
    store.on('sendChatMessage', () => {
      emitCount++
    })
    store.send({ type: 'TRANSCRIBE_OK', text: 'irrelevant' })
    expect(store.getSnapshot().context.state).toBe('idle')
    expect(emitCount).toBe(0)
  })

  it('no-op: STOP_AGENT from listening does not change state or emit', () => {
    const store = createVoiceLoopStore()
    let emitCount = 0
    store.on('cancelChatStream', () => {
      emitCount++
    })
    store.send({ type: 'OPEN' })
    store.send({ type: 'STOP_AGENT' })
    expect(store.getSnapshot().context.state).toBe('listening')
    expect(emitCount).toBe(0)
  })

  it('OPEN after CLOSE returns to listening', () => {
    const store = createVoiceLoopStore()
    store.send({ type: 'OPEN' })
    store.send({ type: 'CLOSE' })
    expect(store.getSnapshot().context.state).toBe('closed')
    store.send({ type: 'OPEN' })
    expect(store.getSnapshot().context.state).toBe('listening')
  })
})
