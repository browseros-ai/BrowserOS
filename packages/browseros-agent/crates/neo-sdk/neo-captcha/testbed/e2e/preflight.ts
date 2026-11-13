import {
  McpSession,
  type McpToolInfo,
} from '../../../../../contracts/claw-mcp/tests/mcp-client'

/** `sdk`: neo-server's browseros-claw-sdk (MCP `run`). `claw-server`: the desktop server. */
export type Server = 'sdk' | 'claw-server'

export interface Preflight {
  url: string
  server: Server
  tools: string[]
  /** Whether the solver is offered: `neo.solveCaptcha` in `run`'s description, or `solve_captcha`. */
  solverListed: boolean | null
  health: unknown
  ok: boolean
  /** Nothing can be tested until the setup changes: exit 2. */
  blocker?: string
  /** An expectation failed before any page ran: exit 1. */
  error?: string
}

/** The owner's own Neo and dev ports, which a dev-watch run must never reuse. */
export const OWNER_PORTS = new Set([
  9010, 9011, 9121, 9211, 9212, 9213, 9214, 9215,
])
const SDK_SOLVER = 'neo.solveCaptcha('
const NO_KEY = {
  sdk: 'neo.solveCaptcha is not offered: this Neo has no readable CAPTCHA provider key (BROWSEROS_CAPSOLVER_API_KEY_FILE or BROWSEROS_2CAPTCHA_API_KEY_FILE), so every call fails host/captcha_unavailable. Give it a key (--keys placeholder needs no account), or pass --expect-tool-missing to test its absence.',
  'claw-server':
    'solve_captcha is not offered: claw-server needs a CAPTCHA provider key and hosted mode. Give it a key, or pass --expect-tool-missing to test its absence.',
}

export function connected(health: unknown): boolean {
  const value = health as {
    status?: string
    browser?: { connected?: boolean }
  } | null
  return value?.status === 'ok' && value.browser?.connected === true
}

export async function fetchHealth(url: string): Promise<unknown> {
  const response = await fetch(new URL('/system/health', url), {
    signal: AbortSignal.timeout(5000),
  })
  if (!response.ok) throw new Error(`health HTTP ${response.status}`)
  return response.json()
}

async function listTools(url: string): Promise<McpToolInfo[]> {
  const session = await McpSession.connect(
    url.replace(/\/mcp\/?$/, ''),
    'captcha-preflight',
  )
  try {
    return await session.listTools()
  } finally {
    await session.close().catch(() => {})
  }
}

/**
 * Proves, before any page opens, that the endpoint is the server under test, that its browser
 * is connected, and that the solver is offered (or, with `expectToolMissing`, that it is not).
 */
export async function checkPreflight(
  url: string,
  options: {
    server: Server
    expectToolMissing?: boolean
    refuseOwnerPorts?: boolean
  },
  dependencies = {
    health: () => fetchHealth(url),
    tools: () => listTools(url),
  },
): Promise<Preflight> {
  const result: Preflight = {
    url,
    server: options.server,
    tools: [],
    solverListed: null,
    health: null,
    ok: false,
  }
  const port = Number(new URL(url).port)
  if (options.refuseOwnerPorts && OWNER_PORTS.has(port)) {
    result.blocker = `refusing owner port ${port}`
    return result
  }
  try {
    result.health = await dependencies.health()
    if (!connected(result.health)) {
      result.blocker = 'health must be ok with the browser connected'
      return result
    }
    const tools = await dependencies.tools()
    result.tools = tools.map((tool) => tool.name)
    const named = (name: string) => result.tools.includes(name)
    if (options.server === 'sdk' && (!named('run') || named('tabs'))) {
      result.blocker = named('tabs')
        ? `the endpoint is claw-server (tools: ${result.tools.join(', ')}), not the Neo SDK; pass --server claw-server to test claw-server`
        : `the endpoint has no run tool (tools: ${result.tools.join(', ') || 'none'})`
      return result
    }
    if (options.server === 'claw-server' && !named('tabs')) {
      result.blocker = named('run')
        ? 'the endpoint is the Neo SDK (one tool: run); drop --server claw-server'
        : `the endpoint has no tabs tool (tools: ${result.tools.join(', ') || 'none'})`
      return result
    }
    result.solverListed =
      options.server === 'sdk'
        ? (
            tools.find((tool) => tool.name === 'run')?.description ?? ''
          ).includes(SDK_SOLVER)
        : named('solve_captcha')
    if (!result.solverListed && !options.expectToolMissing) {
      result.blocker = NO_KEY[options.server]
      return result
    }
    if (result.solverListed && options.expectToolMissing) {
      result.error = `${options.server === 'sdk' ? 'neo.solveCaptcha' : 'solve_captcha'} is offered but --expect-tool-missing was set`
      return result
    }
    result.ok = true
  } catch (error) {
    result.blocker = `preflight request failed: ${error instanceof Error ? error.message : String(error)}`
  }
  return result
}

export function renderPreflight(preflight: Preflight): string {
  const problem = preflight.blocker
    ? `\n\nSETUP BLOCKER: ${preflight.blocker}`
    : preflight.error
      ? `\n\nPREFLIGHT FAILED: ${preflight.error}`
      : ''
  return `Preflight: ${JSON.stringify(preflight)}${problem}\n\n`
}
