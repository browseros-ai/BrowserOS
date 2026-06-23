import { z } from 'zod'
import { defineTool, errorResult, textResult } from './framework'

const DEFAULT_TIMEOUT_MS = 30_000

const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor as new (
  ...args: string[]
) => (...injected: unknown[]) => Promise<unknown>

const DESCRIPTION = `Run JavaScript against the \`browser\` SDK in the server runtime for multi-step flows and data extraction that would otherwise take many tool calls. \`console.log\` is captured; \`return\` a value to read it back; exceptions come back as a result, not a thrown error.

Available as \`browser\`:
  browser.pages.list() / newPage(url) / close(pageId) / getInfo(pageId)
  browser.observe(pageId).snapshot()  -> { text, refs }
  browser.observe(pageId).diff()      -> { text, added, removed, changed }
  browser.observe(pageId).resolveRef(ref)
  browser.input(pageId).click(ref) / fill(ref,value) / type(text) / press(key) / hover(ref) / selectOption(ref,value) / scroll(dir,amount,ref?)
  browser.nav(pageId).goto(url) / back() / forward() / reload()
  browser.cdp(method, params?, sessionId?)   // raw CDP escape hatch
Refs (eN) come from a snapshot's text/refs.`

interface RunOutcome {
  ok: boolean
  value: unknown
  logs: string[]
  error?: Error
}

export const run = defineTool({
  name: 'run',
  description: DESCRIPTION,
  input: z.object({
    code: z
      .string()
      .describe(
        'Async-capable JS body. Use top-level await; `return` a value.',
      ),
    timeout: z
      .number()
      .optional()
      .describe('Max run time in ms (default 30000).'),
  }),
  annotations: { openWorldHint: true },
  handler: async (args, ctx) => {
    let fn: (...injected: unknown[]) => Promise<unknown>
    try {
      fn = new AsyncFunction(
        'browser',
        'console',
        `"use strict";\n${args.code}`,
      )
    } catch (err) {
      return errorResult(
        `run: syntax error - ${err instanceof Error ? err.message : String(err)}`,
      )
    }

    const logs: string[] = []
    const captured = makeConsole(logs)
    const outcome = await execute(
      fn,
      ctx.session,
      captured,
      args.timeout ?? DEFAULT_TIMEOUT_MS,
      logs,
    )
    return outcome.ok
      ? textResult(format(outcome), { ok: true })
      : { ...errorResult(format(outcome)), structuredContent: { ok: false } }
  },
})

/** Runs injected agent code and converts script failures into tool results. */
async function execute(
  fn: (...injected: unknown[]) => Promise<unknown>,
  browser: unknown,
  console: Console,
  timeoutMs: number,
  logs: string[],
): Promise<RunOutcome> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new Error(`run exceeded ${timeoutMs}ms`)),
      timeoutMs,
    )
  })
  try {
    const value = await Promise.race([fn(browser, console), timeout])
    return { ok: true, value, logs }
  } catch (err) {
    return {
      ok: false,
      value: undefined,
      logs,
      error: err instanceof Error ? err : new Error(String(err)),
    }
  } finally {
    if (timer) clearTimeout(timer)
  }
}

function makeConsole(logs: string[]): Console {
  const sink =
    (level: string) =>
    (...parts: unknown[]) => {
      logs.push(
        `${level}${parts.map((part) => (typeof part === 'string' ? part : safeStringify(part))).join(' ')}`,
      )
    }
  return {
    log: sink(''),
    info: sink(''),
    warn: sink('warn: '),
    error: sink('error: '),
    debug: sink(''),
  } as unknown as Console
}

function format(outcome: RunOutcome): string {
  const sections: string[] = []
  if (outcome.error) {
    sections.push(`error: ${outcome.error.message}`)
  } else {
    sections.push('ok')
    if (outcome.value !== undefined) {
      sections.push(`return: ${safeStringify(outcome.value)}`)
    }
  }
  if (outcome.logs.length > 0) {
    sections.push(`logs:\n${outcome.logs.join('\n')}`)
  }
  return sections.join('\n')
}

function safeStringify(value: unknown): string {
  if (value === undefined) return 'undefined'
  try {
    return JSON.stringify(value, null, 2) ?? String(value)
  } catch {
    return String(value)
  }
}
