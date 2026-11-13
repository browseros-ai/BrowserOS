import { describe, expect, test } from 'bun:test'
import {
  agentTaskId,
  clientInfoOf,
  isRunCall,
  readRequestBody,
} from './request-body'

const encode = (text: string) => new TextEncoder().encode(text)

describe('readRequestBody', () => {
  test('reads the agent task id, the client and whether the call is a run', () => {
    const body = readRequestBody(
      encode(
        JSON.stringify({
          jsonrpc: '2.0',
          id: 'a',
          method: 'tools/call',
          params: {
            name: 'run',
            _meta: {
              'com.browseros.neo/run': { taskId: 'zuzu-1' },
              'io.modelcontextprotocol/clientInfo': {
                name: 'claude-code',
                version: '2.1.322',
              },
            },
          },
        }),
      ),
    )
    const message = body.messages[0]
    if (!message) throw new Error('no message')
    expect(isRunCall(message)).toBe(true)
    expect(agentTaskId(message)).toBe('zuzu-1')
    expect(clientInfoOf(message)).toEqual({
      name: 'claude-code',
      version: '2.1.322',
    })
  })

  test('an invalid task id is not the agent naming its task', () => {
    const body = readRequestBody(
      encode(
        JSON.stringify({
          id: 1,
          method: 'tools/call',
          params: {
            _meta: { 'com.browseros.neo/run': { taskId: 'x'.repeat(129) } },
          },
        }),
      ),
    )
    expect(body.messages[0] && agentTaskId(body.messages[0])).toBeNull()
  })

  test('a body that is not UTF-8 JSON yields no messages, so it is forwarded untouched', () => {
    expect(
      readRequestBody(new Uint8Array([0xff, 0xfe, 0x7b])).messages,
    ).toEqual([])
    expect(readRequestBody(encode('{"id":')).messages).toEqual([])
  })
})
