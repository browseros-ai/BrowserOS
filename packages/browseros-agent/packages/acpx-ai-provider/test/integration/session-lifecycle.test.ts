import { describe, expect, test } from 'bun:test'
import type { LanguageModelV2CallOptions } from '@ai-sdk/provider'
import { createAcpxProvider } from '../../src/index.ts'
import { acpResult } from '../helpers/acp-event-builders.ts'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime.ts'

const HISTORY: LanguageModelV2CallOptions['prompt'] = [
  { role: 'system', content: 'sys' },
  { role: 'user', content: [{ type: 'text', text: 'first' }] },
  { role: 'assistant', content: [{ type: 'text', text: 'reply' }] },
  { role: 'user', content: [{ type: 'text', text: 'second' }] },
]

const noopScripts = (count: number) =>
  Array.from({ length: count }, () => ({
    events: [],
    result: acpResult.completed('end_turn'),
  }))

async function drain(stream: ReadableStream<unknown>): Promise<void> {
  const reader = stream.getReader()
  while (true) {
    const { done } = await reader.read()
    if (done) return
  }
}

describe('session reuse', () => {
  test('two consecutive calls with the same key call ensureSession only once', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(2) })
    const provider = createAcpxProvider({ agent: 'claude', cwd: '/r', runtime })
    const model = provider.languageModel()
    await drain((await model.doStream({ prompt: HISTORY })).stream)
    await drain((await model.doStream({ prompt: HISTORY })).stream)
    expect(runtime.ensureSessionCalls).toHaveLength(1)
  })

  test('different sessionKeys produce separate ensureSession calls', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(2) })
    const provider = createAcpxProvider({ agent: 'claude', cwd: '/r', runtime })
    const a = provider.languageModel(undefined, { sessionKey: 'a' })
    const b = provider.languageModel(undefined, { sessionKey: 'b' })
    await drain((await a.doStream({ prompt: HISTORY })).stream)
    await drain((await b.doStream({ prompt: HISTORY })).stream)
    expect(runtime.ensureSessionCalls.map((c) => c.sessionKey)).toEqual([
      'a',
      'b',
    ])
  })
})

describe('fresh vs continuation', () => {
  test('first call sends full history; subsequent only the latest user message', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(2) })
    const provider = createAcpxProvider({ agent: 'claude', cwd: '/r', runtime })
    const model = provider.languageModel()
    await drain((await model.doStream({ prompt: HISTORY })).stream)
    await drain((await model.doStream({ prompt: HISTORY })).stream)

    const firstText = runtime.startTurnCalls[0]?.text ?? ''
    const secondText = runtime.startTurnCalls[1]?.text ?? ''
    expect(firstText).toContain('System: sys')
    expect(firstText).toContain('User: first')
    expect(firstText).toContain('User: second')
    expect(secondText).toBe('User: second')
  })

  test('close() resets the fresh flag so the next call sends full history again', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(3) })
    const provider = createAcpxProvider({ agent: 'claude', cwd: '/r', runtime })
    const model = provider.languageModel()
    await drain((await model.doStream({ prompt: HISTORY })).stream)
    await provider.close()
    await drain((await model.doStream({ prompt: HISTORY })).stream)

    const thirdText = runtime.startTurnCalls[1]?.text ?? ''
    expect(thirdText).toContain('System: sys')
    expect(thirdText).toContain('User: first')
  })
})

describe('runtime injection and lifecycle methods', () => {
  test('provided runtime is used directly without createAcpRuntime', () => {
    const runtime = new MockAcpRuntime()
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    expect(provider.runtime).toBe(runtime)
  })

  test('cancel calls runtime.cancel for every cached handle', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(2) })
    const provider = createAcpxProvider({ agent: 'claude', cwd: '/r', runtime })
    await drain(
      (
        await provider.languageModel(undefined, { sessionKey: 'a' }).doStream({
          prompt: HISTORY,
        })
      ).stream,
    )
    await drain(
      (
        await provider.languageModel(undefined, { sessionKey: 'b' }).doStream({
          prompt: HISTORY,
        })
      ).stream,
    )
    await provider.cancel('user')
    expect(runtime.cancelCalls).toHaveLength(2)
    expect(runtime.cancelCalls.every((c) => c.reason === 'user')).toBe(true)
  })

  test('close clears handle cache and calls runtime.close', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(2) })
    const provider = createAcpxProvider({ agent: 'claude', cwd: '/r', runtime })
    await drain(
      (await provider.languageModel().doStream({ prompt: HISTORY })).stream,
    )
    await provider.close('done')
    expect(runtime.closeCalls).toHaveLength(1)
    expect(runtime.closeCalls[0]?.reason).toBe('done')
  })

  test('setMode and setConfigOption fan out to every cached handle', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(2) })
    const provider = createAcpxProvider({ agent: 'claude', cwd: '/r', runtime })
    await drain(
      (
        await provider.languageModel(undefined, { sessionKey: 'a' }).doStream({
          prompt: HISTORY,
        })
      ).stream,
    )
    await drain(
      (
        await provider.languageModel(undefined, { sessionKey: 'b' }).doStream({
          prompt: HISTORY,
        })
      ).stream,
    )
    await provider.setMode('plan')
    await provider.setConfigOption('model', 'opus')
    expect(runtime.setModeCalls).toHaveLength(2)
    expect(runtime.setConfigOptionCalls).toHaveLength(2)
  })

  test('doctor returns the runtime report', async () => {
    const runtime = new MockAcpRuntime({
      doctorReport: { ok: false, message: 'no agent', code: 'AGENT_MISSING' },
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    expect(await provider.doctor()).toMatchObject({
      ok: false,
      code: 'AGENT_MISSING',
    })
  })

  test('prepare() ensures session without sending a turn', async () => {
    const runtime = new MockAcpRuntime()
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    const handle = await provider.prepare()
    expect(handle).toBeDefined()
    expect(runtime.ensureSessionCalls).toHaveLength(1)
    expect(runtime.startTurnCalls).toHaveLength(0)
  })

  test('per-call agent override threads through to ensureSession', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(1) })
    const provider = createAcpxProvider({ agent: 'claude', cwd: '/r', runtime })
    const codex = provider.languageModel(undefined, {
      agent: 'codex',
      sessionKey: 'codex-key',
    })
    await drain((await codex.doStream({ prompt: HISTORY })).stream)
    expect(runtime.ensureSessionCalls[0]?.agent).toBe('codex')
  })
})

describe('abort signal', () => {
  test('threads abortSignal into runtime.startTurn', async () => {
    const runtime = new MockAcpRuntime({ turnScripts: noopScripts(1) })
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    const controller = new AbortController()
    await drain(
      (
        await provider.languageModel().doStream({
          prompt: HISTORY,
          abortSignal: controller.signal,
        })
      ).stream,
    )
    expect(runtime.startTurnCalls[0]?.signal).toBe(controller.signal)
  })
})
