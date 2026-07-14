import { mkdtempSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { TEST_PORTS } from '@browseros/shared/constants/ports'

const DEFAULT_BINARY_PATH =
  process.env.BROWSEROS_BINARY ??
  '/Applications/BrowserOS.app/Contents/MacOS/BrowserOS'
const PORT_SCAN_RANGE = 100

export interface RuntimePorts {
  cdp: number
  server: number
  extension: number
}

export interface TestRuntimePlan {
  ports: RuntimePorts
  userDataDir: string
  binaryPath: string
  headless: boolean
  extraArgs: string[]
  usesFixedPorts: boolean
}

function parseExtraArgs(value: string | undefined): string[] {
  if (!value) {
    return []
  }
  return value
    .split(/\s+/)
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
}

function parsePort(
  value: string | undefined,
  envName: string,
): number | undefined {
  if (!value) {
    return undefined
  }

  const parsed = Number.parseInt(value, 10)
  if (!Number.isInteger(parsed) || parsed <= 0 || parsed > 65535) {
    throw new Error(`Invalid ${envName} value: ${value}`)
  }
  return parsed
}

async function isPortAvailable(port: number): Promise<boolean> {
  return await new Promise<boolean>((resolve) => {
    const server = createServer()
    server.unref()
    server.once('error', () => resolve(false))
    server.listen(port, () => {
      server.close(() => resolve(true))
    })
  })
}

async function findAvailablePort(
  startPort: number,
  reserved: Set<number>,
): Promise<number> {
  for (let port = startPort; port < startPort + PORT_SCAN_RANGE; port++) {
    if (reserved.has(port)) {
      continue
    }
    if (await isPortAvailable(port)) {
      reserved.add(port)
      return port
    }
  }
  throw new Error(`Failed to find available port near ${startPort}`)
}

function resolveFixedPort(
  testEnvName:
    | 'BROWSEROS_TEST_CDP_PORT'
    | 'BROWSEROS_TEST_SERVER_PORT'
    | 'BROWSEROS_TEST_EXTENSION_PORT',
  baseEnvName:
    | 'BROWSEROS_CDP_PORT'
    | 'BROWSEROS_SERVER_PORT'
    | 'BROWSEROS_EXTENSION_PORT',
): number | undefined {
  const testPort = parsePort(process.env[testEnvName], testEnvName)
  if (testPort !== undefined) {
    return testPort
  }
  if (process.env.BROWSEROS_TEST_USE_ENV_PORTS === 'true') {
    return parsePort(process.env[baseEnvName], baseEnvName)
  }
  return undefined
}

function assertUniquePorts(ports: RuntimePorts): void {
  const values = new Set([ports.cdp, ports.server, ports.extension])
  if (values.size !== 3) {
    throw new Error(
      `Port conflict detected: cdp=${ports.cdp} server=${ports.server} extension=${ports.extension}`,
    )
  }
}

export async function resolveRuntimePorts(): Promise<{
  ports: RuntimePorts
  usesFixedPorts: boolean
}> {
  const cdpOverride = resolveFixedPort(
    'BROWSEROS_TEST_CDP_PORT',
    'BROWSEROS_CDP_PORT',
  )
  const serverOverride = resolveFixedPort(
    'BROWSEROS_TEST_SERVER_PORT',
    'BROWSEROS_SERVER_PORT',
  )
  const extensionOverride = resolveFixedPort(
    'BROWSEROS_TEST_EXTENSION_PORT',
    'BROWSEROS_EXTENSION_PORT',
  )

  const reserved = new Set<number>()
  const cdp = cdpOverride ?? (await findAvailablePort(TEST_PORTS.cdp, reserved))
  reserved.add(cdp)
  const server =
    serverOverride ?? (await findAvailablePort(TEST_PORTS.server, reserved))
  reserved.add(server)
  const extension =
    extensionOverride ??
    (await findAvailablePort(TEST_PORTS.extension, reserved))

  const ports = { cdp, server, extension }
  assertUniquePorts(ports)

  return {
    ports,
    usesFixedPorts:
      cdpOverride !== undefined ||
      serverOverride !== undefined ||
      extensionOverride !== undefined,
  }
}

/** Resolves test environment settings into an explicit browser launch plan. */
export async function createTestRuntimePlan(): Promise<TestRuntimePlan> {
  const resolvedPorts = await resolveRuntimePorts()
  const userDataDir = mkdtempSync(join(tmpdir(), 'browseros-test-'))
  const headless = process.env.BROWSEROS_TEST_HEADLESS !== 'false'
  const extraArgs = parseExtraArgs(process.env.BROWSEROS_TEST_EXTRA_ARGS)

  return {
    ports: resolvedPorts.ports,
    userDataDir,
    binaryPath: DEFAULT_BINARY_PATH,
    headless,
    extraArgs,
    usesFixedPorts: resolvedPorts.usesFixedPorts,
  }
}
