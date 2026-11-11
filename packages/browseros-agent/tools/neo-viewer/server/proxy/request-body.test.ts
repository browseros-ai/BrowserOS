import { describe, expect, test } from 'bun:test'
import {
  agentTaskId,
  applyEdits,
  clientInfoOf,
  isRunCall,
  readRequestBody,
  taskIdEdit,
} from './request-body'

const encode = (text: string) => new TextEncoder().encode(text)

/** The body after stamping `task-1` into its first message, or null when it stays as sent. */
function stamped(text: string): string | null {
  const body = readRequestBody(encode(text))
  const message = body.messages[0]
  if (!message || body.text === null) throw new Error('no message')
  const edit = taskIdEdit(body.text, message, 'task-1')
  return edit ? applyEdits(body.text, [edit]) : null
}

describe('taskIdEdit', () => {
  test('adds _meta when the call has none, and changes no other byte', () => {
    const text =
      '{ "jsonrpc":"2.0", "id": 3, "method":"tools/call",\n  "params": {"name":"run","arguments":{"code":"return \\u00e9 + 1e3"}} }'
    expect(stamped(text)).toBe(
      '{ "jsonrpc":"2.0", "id": 3, "method":"tools/call",\n  "params": {"_meta":{"com.browseros.neo/run":{"taskId":"task-1"}},"name":"run","arguments":{"code":"return \\u00e9 + 1e3"}} }',
    )
  })

  test('adds the run key to an existing _meta', () => {
    const text =
      '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"progressToken":1}}}'
    expect(stamped(text)).toBe(
      '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":{"taskId":"task-1"},"progressToken":1}}}',
    )
  })

  test('adds taskId beside the agent requestId, into an empty run meta, and over a null', () => {
    expect(
      stamped(
        '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":{"requestId":"r"}}}}',
      ),
    ).toBe(
      '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":{"taskId":"task-1","requestId":"r"}}}}',
    )
    expect(
      stamped(
        '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":{ }}}}',
      ),
    ).toBe(
      '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":{"taskId":"task-1" }}}}',
    )
    expect(
      stamped(
        '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":{"taskId":null}}}}',
      ),
    ).toBe(
      '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":{"taskId":"task-1"}}}}',
    )
  })

  test('leaves a call alone when it names a task, or its meta is not an object', () => {
    expect(
      stamped(
        '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":{"taskId":"own"}}}}',
      ),
    ).toBeNull()
    expect(
      stamped(
        '{"id":1,"method":"tools/call","params":{"name":"run","_meta":{"com.browseros.neo/run":7}}}',
      ),
    ).toBeNull()
    expect(
      stamped(
        '{"id":1,"method":"tools/call","params":{"name":"run","_meta":[]}}',
      ),
    ).toBeNull()
  })

  test('stamps each run in a batch', () => {
    const text =
      '[{"id":1,"method":"tools/call","params":{"name":"run"}},{"id":2,"method":"tools/call","params":{}}]'
    const body = readRequestBody(encode(text))
    const edits = body.messages.flatMap((message) => {
      const edit = body.text ? taskIdEdit(body.text, message, 't') : null
      return edit ? [edit] : []
    })
    expect(applyEdits(text, edits)).toBe(
      '[{"id":1,"method":"tools/call","params":{"_meta":{"com.browseros.neo/run":{"taskId":"t"}},"name":"run"}},{"id":2,"method":"tools/call","params":{"_meta":{"com.browseros.neo/run":{"taskId":"t"}}}}]',
    )
  })
})

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
