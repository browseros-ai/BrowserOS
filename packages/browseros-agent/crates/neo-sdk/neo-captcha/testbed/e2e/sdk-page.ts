import {
  type McpSession,
  textOf,
} from '../../../../../contracts/claw-mcp/tests/mcp-client'
import type { SiteCheck } from '../fixtures/catalog'
import type { PageSession, Timeouts } from './page-session'
import { bodyProbe, observationProbe, readyProbe, submitProbe } from './probes'
import { notSolved, parseSdkSolve } from './solve-result'

/** A failed `run`: the SDK's `{ owner, code, message }`. */
export class SdkRunError extends Error {
  constructor(
    readonly owner: string,
    readonly code: string,
    message: string,
  ) {
    super(message)
  }
}

const TIMEOUTS = new Set(['wait_timeout', 'run_timeout'])
const field = (value: unknown, name: string): unknown =>
  value !== null && typeof value === 'object'
    ? (value as Record<string, unknown>)[name]
    : undefined

/** Opens every script, so the local stub can answer it without a JavaScript engine. */
export function stepTag(step: string, args: Record<string, unknown> = {}) {
  return `/* captcha-e2e:${step} ${JSON.stringify(args).replaceAll('*/', '*\\/')} */`
}
export function parseStepTag(code: string) {
  const match = code.match(/^\/\* captcha-e2e:([a-z]+) (\{.*?\}) \*\//)
  if (!match) return null
  return {
    step: match[1] as string,
    args: JSON.parse(match[2] as string) as Record<string, unknown>,
  }
}

/** One `run`: the script's value, or its error as an `SdkRunError`. */
export async function runScript(
  mcp: McpSession,
  code: string,
  timeoutMs: number,
): Promise<unknown> {
  const result = await mcp.callTool(
    'run',
    { code, timeout: timeoutMs },
    timeoutMs + 15_000,
  )
  const structured = result.structuredContent
  if (result.isError) {
    const error = field(structured, 'error')
    const text = (value: unknown, fallback: string) =>
      typeof value === 'string' ? value : fallback
    throw new SdkRunError(
      text(field(error, 'owner'), 'neo'),
      text(field(error, 'code'), 'run_failed'),
      text(field(error, 'message'), textOf(result)),
    )
  }
  if (structured && 'value' in structured) return structured.value
  return JSON.parse(textOf(result))
}

/** A page over the Neo SDK's one tool, `run`: a task per page, closed at the end. */
export function sdkPage(
  mcp: McpSession,
  id: string,
  timeouts: Timeouts,
): PageSession {
  let task: string | undefined
  let tab: number | undefined
  const run = (code: string, timeoutMs = timeouts.probeMs) =>
    runScript(mcp, code, timeoutMs)
  const script = (step: string, body: string, args = {}) =>
    `${stepTag(step, { task, tab, ...args })}
const page = (await neo.task(${JSON.stringify(task)})).page(${tab});
${body}`
  return {
    async open(url) {
      const opened = await run(`${stepTag('open', { id })}
const task = await neo.createTask(${JSON.stringify(`captcha-e2e ${id}`)});
const page = await task.newPage();
return { taskId: task.id, tabId: page.tabId };`)
      const taskId = field(opened, 'taskId')
      const tabId = field(opened, 'tabId')
      if (typeof taskId !== 'string' || !Number.isInteger(tabId))
        throw new Error('the SDK returned no task or tab id')
      task = taskId
      tab = tabId as number
      await run(
        script(
          'goto',
          `await page.goto(${JSON.stringify(url)}, { timeout: ${timeouts.navigationMs} });
return true;`,
          { url },
        ),
        timeouts.navigationMs + 5_000,
      )
    },
    async ready(expression) {
      try {
        await run(
          script(
            'ready',
            `await page.waitForFunction(() => { ${readyProbe(expression)} }, null, { timeout: ${timeouts.probeMs} });
return true;`,
          ),
          timeouts.probeMs + 5_000,
        )
        return true
      } catch (error) {
        if (error instanceof SdkRunError && TIMEOUTS.has(error.code))
          return false
        throw error
      }
    },
    async solve() {
      try {
        return parseSdkSolve(
          await run(
            script('solve', 'return await neo.solveCaptcha(page);'),
            timeouts.solveMs,
          ),
        )
      } catch (error) {
        const owner = error instanceof SdkRunError ? error.owner : 'mcp'
        const code =
          error instanceof SdkRunError ? error.code : 'request_failed'
        return {
          ...notSolved(),
          verdict: 'error',
          error: `${owner}/${code}`,
          message: (error instanceof Error ? error.message : '').slice(0, 300),
        }
      }
    },
    async observe(tokenExpression) {
      return await run(
        script(
          'observe',
          `return await page.evaluate(() => { ${observationProbe(tokenExpression)} });`,
        ),
      )
    },
    async submit(site: SiteCheck) {
      // The form's own click, as claw-server's runner did: a challenge frame still drawn over
      // the button cannot swallow it.
      await run(
        script(
          'submit',
          `return await page.evaluate(() => { ${submitProbe(site.submitSelector)} });`,
        ),
      )
      try {
        const found = await run(
          script(
            'site',
            `const probe = () => { ${bodyProbe(site)} };
await page.waitForFunction(probe, null, { timeout: ${timeouts.probeMs} });
return await page.evaluate(probe);`,
          ),
          timeouts.probeMs + 5_000,
        )
        return found === 'accepted' || found === 'rejected' ? found : null
      } catch (error) {
        if (error instanceof SdkRunError && TIMEOUTS.has(error.code))
          return null
        throw error
      }
    },
    async close() {
      if (task === undefined) return
      await run(`${stepTag('close', { task })}
await (await neo.task(${JSON.stringify(task)})).close();
return true;`)
    },
  }
}
