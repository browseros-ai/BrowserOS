import { expect, test } from 'bun:test'
import { createAcpxProvider } from '../../src/index'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime'

test('compact uses the selected agent for command lookup and execution', async () => {
  const runtime = new MockAcpRuntime({
    turnScripts: [
      {
        events: [
          {
            type: 'status',
            text: '',
            tag: 'available_commands_update',
            availableCommands: [
              {
                name: '/condense',
                description: 'Compact context',
                hasInput: false,
              },
            ],
          },
        ],
        result: { status: 'completed', stopReason: 'end_turn' },
      },
      { events: [], result: { status: 'completed', stopReason: 'end_turn' } },
    ],
  })
  // Leave sessionKey unspecified so the provider derives it from the agent.
  // An explicit key would hide a lookup accidentally using the default agent.
  const provider = createAcpxProvider({
    agent: 'claude',
    cwd: '/tmp/test',
    runtime,
  })
  await provider.languageModel(undefined, { agent: 'codex' }).doGenerate({
    prompt: [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }],
  })

  await provider.compact({ agent: 'codex' })

  expect(runtime.startTurnCalls.at(-1)?.text).toBe('/condense')
  expect(runtime.startTurnCalls.at(-1)?.handle).toEqual(
    runtime.startTurnCalls[0]?.handle,
  )
  expect(runtime.ensureSessionCalls).toHaveLength(1)
  expect(runtime.ensureSessionCalls[0]?.agent).toBe('codex')
  await provider.close()
})
