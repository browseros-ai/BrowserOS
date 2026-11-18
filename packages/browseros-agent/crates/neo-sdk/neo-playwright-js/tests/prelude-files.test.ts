// setInputFiles in the prelude: upstream's client and channel checks, with the messages and
// error names measured against playwright-core 1.64.1 before the old differential tiers
// were retired, and what reaches Rust when the checks pass.
import { expect, it } from 'bun:test'
import { type Answer, loadPrelude, SEED } from './support/prelude-runtime'

/** Runs `call` and answers what it threw, and how many setInputFiles ops it sent. */
async function attempt(code: string, answer?: Answer) {
  const runtime = loadPrelude({ seed: SEED, ...(answer ? { answer } : {}) })
  const outcome = await runtime.run(
    `try { await ${code}; return { returned: true } } catch (e) { return { name: e.name, message: e.message, code: e.code, delivery: e.delivery } }`,
  )
  return { outcome, sent: runtime.sent('locator.setInputFiles') }
}

const files = (call: { input: Record<string, unknown> }) => call.input.files

it('tests that paths travel as the script wrote them, for Rust to stat', async () => {
  const { outcome, sent } = await attempt(
    "page.setInputFiles('#f', ['a.txt', './dir/../b.txt'])",
  )
  expect(outcome).toEqual({ returned: true })
  expect(sent.map(files)).toEqual([{ paths: ['a.txt', './dir/../b.txt'] }])
})

it("tests that payloads carry upstream's MIME type and its server's base64 of the buffer", async () => {
  const { sent } = await attempt(
    `page.locator('#f').setInputFiles([
      { name: 'a.png', buffer: Buffer.from('png') },
      { name: 'b.txt', mimeType: '', buffer: Buffer.from('b') },
      { name: 'noext', buffer: Buffer.from('c') },
      { name: new String('d.json'), mimeType: 'text/x-d', buffer: Buffer.from('{}') },
      { name: 'e.bin', buffer: new Uint8Array([104, 105]) },
    ])`,
  )
  expect(sent.map(files)).toEqual([
    {
      payloads: [
        { name: 'a.png', mimeType: 'image/png', buffer: 'cG5n' },
        { name: 'b.txt', mimeType: 'text/plain', buffer: 'Yg==' },
        { name: 'noext', mimeType: 'application/octet-stream', buffer: 'Yw==' },
        { name: 'd.json', mimeType: 'text/x-d', buffer: 'e30=' },
        // A plain Uint8Array's toString joins its numbers, and the page's atob refuses that,
        // as upstream's server makes the same text of it.
        {
          name: 'e.bin',
          mimeType: 'application/octet-stream',
          buffer: '104,105',
        },
      ],
    },
  ])
  const cleared = await attempt("page.setInputFiles('#f', [])")
  expect(cleared.sent.map(files)).toEqual([{ payloads: [] }])
})

it("tests that the client's own refusals are unprefixed and send nothing", async () => {
  for (const [code, name, message] of [
    [
      "page.setInputFiles('#f', ['a.txt', { name: 'x', buffer: Buffer.from('x') }])",
      'Error',
      'File paths cannot be mixed with buffers',
    ],
    [
      "page.setInputFiles('#f', { name: 'big.bin', buffer: new Uint8Array(50 * 1024 * 1024) })",
      'Error',
      'Cannot set buffer larger than 50Mb, please write it to a file and pass its path instead.',
    ],
    [
      "page.setInputFiles('#f')",
      'TypeError',
      "Cannot read properties of undefined (reading 'buffer')",
    ],
    [
      "page.setInputFiles('#f', [{ name: 'x', buffer: Buffer.from('x') }, null])",
      'TypeError',
      "Cannot read properties of null (reading 'buffer')",
    ],
  ] as const) {
    const { outcome, sent } = await attempt(code)
    expect({ code, outcome }).toMatchObject({
      code,
      outcome: { name, message, delivery: 'not_sent' },
    })
    expect(sent).toEqual([])
  }
})

it("tests that Neo refuses a payload of 10 MiB with upstream's advice, after the validator", async () => {
  const limit = 10 * 1024 * 1024
  const refused = await attempt(
    `page.setInputFiles('#f', [{ name: 'a.bin', buffer: Buffer.alloc(${limit / 2}) }, { name: 'b.bin', buffer: new Uint8Array(${limit / 2}) }])`,
  )
  expect(refused.outcome).toMatchObject({
    name: 'Error',
    message:
      'Cannot set buffer larger than 10Mb in Neo, please write it to a file and pass its path instead.',
    delivery: 'not_sent',
  })
  expect(refused.sent).toEqual([])
  // Upstream's validator runs on a payload under 50 MiB first, so its message still wins.
  const invalid = await attempt(
    `page.setInputFiles('#f', { name: 7, buffer: Buffer.alloc(${limit}) })`,
  )
  expect(invalid.outcome).toMatchObject({
    message:
      'page.setInputFiles: payloads[0].name: expected string, got number',
  })
  const under = await attempt(
    `page.setInputFiles('#f', { name: 'a.bin', buffer: Buffer.alloc(${limit - 1}, 'neo') })`,
  )
  expect(under.outcome).toEqual({ returned: true })
  const sent = under.sent.map(files) as { payloads: { buffer: string }[] }[]
  expect(sent[0]?.payloads[0]?.buffer).toBe(
    globalThis.Buffer.alloc(limit - 1, 'neo').toString('base64'),
  )
})

it("tests that the channel validator's payload errors carry the method and send nothing", async () => {
  for (const [code, message] of [
    [
      "page.setInputFiles('#f', { mimeType: 'text/plain', buffer: Buffer.from('x') })",
      'page.setInputFiles: payloads[0].name: expected string, got undefined',
    ],
    [
      "page.setInputFiles('#f', { name: 7, buffer: Buffer.from('x') })",
      'page.setInputFiles: payloads[0].name: expected string, got number',
    ],
    [
      "page.setInputFiles('#f', { name: 'x', mimeType: 7, buffer: Buffer.from('x') })",
      'page.setInputFiles: payloads[0].mimeType: expected string, got number',
    ],
    [
      "page.setInputFiles('#f', { name: 'x', mimeType: null, buffer: Buffer.from('x') })",
      'page.setInputFiles: payloads[0].mimeType: expected string, got object',
    ],
    [
      "page.setInputFiles('#f', { name: 'x', buffer: 'x' })",
      'page.setInputFiles: payloads[0].buffer: expected Buffer, got string',
    ],
    [
      "page.setInputFiles('#f', { name: 'x' })",
      'page.setInputFiles: payloads[0].buffer: expected Buffer, got undefined',
    ],
    [
      "page.setInputFiles('#f', [{ name: 'x', buffer: Buffer.from('x') }, 5])",
      'page.setInputFiles: payloads[1]: expected object, got number',
    ],
    [
      "(await page.$('#f')).setInputFiles({ name: 'x', buffer: 5 })",
      'elementHandle.setInputFiles: payloads[0].buffer: expected Buffer, got number',
    ],
  ] as const) {
    const answer: Answer = (call) =>
      call.name === 'locator.elementHandle'
        ? {
            $neo: 'element',
            page: { $neo: 'page', tab: 1007 },
            seq: 0,
            node: 9,
            display: '<input>',
          }
        : null
    const { outcome, sent } = await attempt(code, answer)
    expect({ code, outcome }).toMatchObject({
      code,
      outcome: { name: 'Error', message, delivery: 'not_sent' },
    })
    expect(sent).toEqual([])
  }
})

it("tests that the client errors Rust raises for it come back unprefixed, the server's not", async () => {
  const failing =
    (message: string): Answer =>
    () => ({
      err: {
        code: 'invalid_input',
        owner: 'script',
        delivery: 'not_sent',
        retryable: false,
        operationId: 'run.1',
        timeout: false,
        message,
      },
    })
  for (const [message, name, seen] of [
    [
      "ENOENT: no such file or directory, stat 'missing.txt'",
      'Error',
      "ENOENT: no such file or directory, stat 'missing.txt'",
    ],
    [
      'Multiple directories are not supported',
      'Error',
      'Multiple directories are not supported',
    ],
    [
      'File paths must be all files or a single directory',
      'Error',
      'File paths must be all files or a single directory',
    ],
    [
      "The argument 'path' must be a string, Uint8Array, or URL without null bytes. Received 'a\\x00b'",
      'TypeError',
      "The argument 'path' must be a string, Uint8Array, or URL without null bytes. Received 'a\\x00b'",
    ],
    [
      "page.setInputFiles: ENOENT: no such file or directory, access '/w/missing.txt'",
      'Error',
      "page.setInputFiles: ENOENT: no such file or directory, access '/w/missing.txt'",
    ],
    [
      'locator.setInputFiles: Error: Node is not an HTMLInputElement',
      'Error',
      'page.setInputFiles: Error: Node is not an HTMLInputElement',
    ],
  ] as const) {
    const { outcome } = await attempt(
      "page.setInputFiles('#f', 'missing.txt')",
      failing(message),
    )
    expect({ message, outcome }).toMatchObject({
      message,
      outcome: { name, message: seen, code: 'invalid_input' },
    })
  }
})
