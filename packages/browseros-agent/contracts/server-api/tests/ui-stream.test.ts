/**
 * The Bun half of the UI stream golden test (CORRECTIONS-1 B2), and the gate for every AI SDK
 * upgrade of the extension.
 *
 * The Rust server's reducer (`crates/browseros-server/src/turn/transcript.rs`) emits the chat
 * stream and stores the message it believes the panel built. This test runs the SDK the
 * extension actually ships (resolved from `apps/app`, not a copy) over each committed golden
 * stream, through the same `DefaultChatTransport` validation and `Chat` reducer `useChat`
 * uses, and asserts the message it builds equals the committed `<case>.message.json`. The Rust
 * half (`tests/turn_golden.rs`) asserts its reducer stores exactly that JSON, so the two
 * reducers agree on every golden; an SDK upgrade that reduces a chunk differently fails here.
 *
 * Run: `bun test contracts/server-api/tests/ui-stream.test.ts` from `packages/browseros-agent`.
 * `BROWSEROS_UPDATE_GOLDENS=1` rewrites the message files from the SDK instead of comparing.
 */
import { describe, expect, test } from 'bun:test'
import assert from 'node:assert/strict'
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const root = join(import.meta.dir, '../../..')
const fixtures = join(root, 'crates/browseros-server/tests/fixtures/ui-stream')
const appDir = join(root, 'apps/app')
const update = process.env.BROWSEROS_UPDATE_GOLDENS !== undefined

// The extension's own install: the gate must run the version the panel runs.
const ai = await import(Bun.resolveSync('ai', appDir))
const react = await import(Bun.resolveSync('@ai-sdk/react', appDir))
const aiVersion: string = JSON.parse(
  readFileSync(join(appDir, 'node_modules/ai/package.json'), 'utf8'),
).version

interface Outcome {
  status: string
  message: unknown
  errorMessage?: string
}

/** One `useChat` turn whose response body is `body`, as the panel would run it. */
async function reduce(body: string): Promise<Outcome> {
  let finished: { message: unknown } | undefined
  let error: Error | undefined
  const chat = new react.Chat({
    id: 'golden',
    transport: new ai.DefaultChatTransport({
      fetch: async () =>
        new Response(new TextEncoder().encode(body), {
          headers: {
            'content-type': 'text/event-stream',
            'x-vercel-ai-ui-message-stream': 'v1',
          },
        }),
    }),
    onFinish: (event: { message: unknown }) => {
      finished = event
    },
    onError: (cause: Error) => {
      error = cause
    },
  })
  await chat.sendMessage({ text: 'Where is Paris?' })
  // The stored form: JSON drops the keys the reducer left `undefined`.
  const message = JSON.parse(JSON.stringify(finished?.message ?? null))
  return { status: chat.status, message, errorMessage: error?.message }
}

/** The `errorText` of the stream's `error` chunk, when it has one. */
function errorText(body: string): string | undefined {
  for (const line of body.split('\n')) {
    if (!line.startsWith('data: {')) continue
    const chunk = JSON.parse(line.slice('data: '.length))
    if (chunk.type === 'error') return chunk.errorText
  }
  return undefined
}

const goldens = readdirSync(fixtures)
  .filter((name) => name.endsWith('.sse') && !name.startsWith('r4-'))
  .map((name) => name.slice(0, -'.sse'.length))
  .sort()

describe(`the golden UI streams under ai@${aiVersion}`, () => {
  test('there are goldens to check', () => {
    expect(goldens.length).toBeGreaterThanOrEqual(8)
  })

  for (const name of goldens) {
    test(name, async () => {
      const body = readFileSync(join(fixtures, `${name}.sse`), 'utf8')
      const outcome = await reduce(body)

      // Every chunk passed the transport's schema: the only error is the stream's own.
      const expectedError = errorText(body)
      expect(outcome.errorMessage).toBe(expectedError)
      expect(outcome.status).toBe(
        expectedError === undefined ? 'ready' : 'error',
      )

      const messagePath = join(fixtures, `${name}.message.json`)
      if (update) {
        writeFileSync(
          messagePath,
          `${JSON.stringify(outcome.message, null, 2)}\n`,
        )
        return
      }
      const committed = JSON.parse(readFileSync(messagePath, 'utf8'))
      assert.deepStrictEqual(
        outcome.message,
        committed,
        'The AI SDK output differs from the committed golden. To regenerate intentionally, run `BROWSEROS_UPDATE_GOLDENS=1 bun test contracts/server-api/tests/ui-stream.test.ts` from packages/browseros-agent and review the diff.',
      )
    })
  }
})

describe('the stream the old server captured (r4)', () => {
  test('every chunk in it is one the extension accepts', async () => {
    const body = readFileSync(join(fixtures, 'r4-ts-response.sse'), 'utf8')
    const outcome = await reduce(body)
    expect(outcome.errorMessage).toBeUndefined()
    expect(outcome.status).toBe('ready')
  })
})
