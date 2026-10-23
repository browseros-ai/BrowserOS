import { afterEach, beforeEach, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const key = 'unit-test-key-must-not-leak-123'
type Vendor = 'capsolver' | '2captcha'
type RequestBody = { clientKey: string; task?: unknown; taskId?: unknown }
type Handler = (
  vendor: Vendor,
  method: string,
  body: RequestBody,
) => unknown | Promise<unknown>
let directory: string
let env: Record<string, string>
let servers: ReturnType<typeof Bun.serve>[]
let calls: { vendor: Vendor; method: string; body: RequestBody }[]
let handler: Handler

function successful(vendor: Vendor, method: string) {
  if (method === 'getBalance') return { errorId: 0, balance: 1.25 }
  if (vendor === '2captcha' && method === 'createTask')
    return { errorId: 0, taskId: 42 }
  return {
    errorId: 0,
    status: 'ready',
    solution:
      vendor === 'capsolver' ? { objects: [0, 4, 7] } : { click: [1, 5, 8] },
  }
}

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), 'captcha-provider-check-'))
  const path = join(directory, 'key')
  await writeFile(path, ` \n${key}\n `)
  env = {
    BROWSEROS_CAPSOLVER_API_KEY_FILE: path,
    BROWSEROS_2CAPTCHA_API_KEY_FILE: path,
  }
  calls = []
  servers = []
  handler = successful
  for (const vendor of ['capsolver', '2captcha'] as const) {
    const server = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      async fetch(request) {
        const method = new URL(request.url).pathname.slice(1)
        const body = (await request.json()) as RequestBody
        calls.push({ vendor, method, body })
        const reply = await handler(vendor, method, body)
        return reply instanceof Response ? reply : Response.json(reply)
      },
    })
    servers.push(server)
    env[`BROWSEROS_${vendor.toUpperCase()}_API_URL`] = server.url.origin
  }
})

afterEach(async () => {
  await Promise.all(servers.map((server) => server.stop(true)))
  await rm(directory, { recursive: true, force: true })
})

function assertLocalSettings() {
  for (const vendor of ['CAPSOLVER', '2CAPTCHA']) {
    expect(env[`BROWSEROS_${vendor}_API_KEY_FILE`]?.startsWith(directory)).toBe(
      true,
    )
    expect(
      servers.some(
        (server) => server.url.origin === env[`BROWSEROS_${vendor}_API_URL`],
      ),
    ).toBe(true)
  }
}

async function run(options = {}) {
  assertLocalSettings()
  const { checkProviders } = await import('./providers')
  return checkProviders({
    env,
    requestTimeoutMs: 200,
    pollIntervalMs: 2,
    pollTimeoutMs: 100,
    ...options,
  })
}

async function cli(args: readonly string[] = []) {
  assertLocalSettings()
  const inherited = { ...process.env }
  for (const vendor of ['CAPSOLVER', '2CAPTCHA']) {
    delete inherited[`BROWSEROS_${vendor}_API_KEY_FILE`]
    delete inherited[`BROWSEROS_${vendor}_API_URL`]
  }
  const child = Bun.spawn(
    [process.execPath, join(import.meta.dir, 'check.ts'), ...args],
    {
      env: { ...inherited, ...env },
      stdout: 'pipe',
      stderr: 'pipe',
    },
  )
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ])
  return { stdout, stderr, exitCode }
}

test('tests that funded vendors run exactly one correctly shaped task', async () => {
  const { report } = await run()
  expect(report.exitCode).toBe(0)
  expect(report.paidTasksCreated).toBe(2)
  expect(report.paidSolvingReady).toEqual({ capsolver: true, '2captcha': true })
  for (const result of report.vendors) {
    expect(result.status).toBe('ok')
    expect(result.balance).toBe(1.25)
    expect(result.task.status).toBe('ok')
    expect(result.task.tiles).toEqual([0, 4, 7])
    expect(result.task.elapsedMs).toBeGreaterThanOrEqual(0)
  }
  expect(calls.every(({ body }) => body.clientKey === key)).toBe(true)
  const tasks = calls.filter(({ method }) => method === 'createTask')
  expect(tasks).toHaveLength(2)
  const image = await Bun.file(join(import.meta.dir, 'sample/grid.png')).bytes()
  const base64 = Buffer.from(image).toString('base64')
  expect(image.length).toBeLessThan(150_000)
  expect(tasks.find(({ vendor }) => vendor === 'capsolver')?.body.task).toEqual(
    {
      type: 'ReCaptchaV2Classification',
      image: base64,
      question: '/m/015qff',
    },
  )
  expect(tasks.find(({ vendor }) => vendor === '2captcha')?.body.task).toEqual({
    type: 'GridTask',
    body: base64,
    comment: 'Select all images with traffic lights',
    rows: 3,
    columns: 3,
  })
})

test('tests that zero balances skip all paid tasks and exit one', async () => {
  handler = () => ({ errorId: 0, balance: 0 })
  const { report } = await run()
  expect(report.exitCode).toBe(1)
  expect(report.paidTasksCreated).toBe(0)
  expect(report.paidSolvingReady).toEqual({
    capsolver: false,
    '2captcha': false,
  })
  expect(
    report.vendors.every(
      (v) => v.status === 'ok' && v.task.status === 'skipped',
    ),
  ).toBe(true)
  expect(calls.map(({ method }) => method)).toEqual([
    'getBalance',
    'getBalance',
  ])
})

test('tests that denied keys retain vendor errors without attempting tasks', async () => {
  handler = () => ({
    errorId: 1,
    errorCode: 'ERROR_KEY_DENIED_ACCESS',
    errorDescription: 'denied',
  })
  const { report } = await run()
  expect(
    report.vendors.every(
      (v) =>
        v.status === 'error' &&
        v.message.includes('ERROR_KEY_DENIED_ACCESS: denied'),
    ),
  ).toBe(true)
  expect(report.exitCode).toBe(1)
  expect(report.paidTasksCreated).toBe(0)
})

test('tests that a missing key fails only its vendor', async () => {
  env.BROWSEROS_CAPSOLVER_API_KEY_FILE = join(directory, 'missing')
  const { report } = await run()
  expect(report.vendors[0]?.status).toBe('error')
  expect(report.vendors[0]?.message).toContain(
    env.BROWSEROS_CAPSOLVER_API_KEY_FILE,
  )
  expect(report.vendors[1]?.task.status).toBe('ok')
  expect(calls.some(({ vendor }) => vendor === 'capsolver')).toBe(false)
})

test('tests that empty key files never reach a vendor', async () => {
  await writeFile(env.BROWSEROS_CAPSOLVER_API_KEY_FILE as string, ' \n ')
  const { report } = await run()
  expect(report.vendors.every((v) => v.status === 'error')).toBe(true)
  expect(report.paidTasksCreated).toBe(0)
  expect(calls).toHaveLength(0)
})

test('tests that 2Captcha processing transitions to ready without resubmission', async () => {
  let polls = 0
  handler = (vendor, method) => {
    if (method === 'getTaskResult' && ++polls === 1)
      return { errorId: 0, status: 'processing' }
    return successful(vendor, method)
  }
  const { report } = await run()
  expect(report.vendors[1]?.task.tiles).toEqual([0, 4, 7])
  expect(polls).toBe(2)
  expect(
    calls.filter((c) => c.vendor === '2captcha' && c.method === 'createTask'),
  ).toHaveLength(1)
  expect(
    calls
      .filter((c) => c.method === 'getTaskResult')
      .every((c) => c.body.taskId === 42),
  ).toBe(true)
})

test('tests that 2Captcha polling stops at its deadline', async () => {
  handler = (vendor, method) =>
    method === 'getTaskResult'
      ? { errorId: 0, status: 'processing' }
      : successful(vendor, method)
  const start = performance.now()
  const { report } = await run({ pollTimeoutMs: 15 })
  expect(report.vendors[1]?.task.status).toBe('error')
  expect(report.vendors[1]?.message).toContain('POLL_TIMEOUT')
  expect(performance.now() - start).toBeLessThan(500)
  expect(report.paidTasksCreated).toBe(2)
  expect(report.exitCode).toBe(1)
})

test('tests that a slow poll request cannot overrun its overall deadline', async () => {
  handler = async (vendor, method) => {
    if (method === 'getTaskResult') await Bun.sleep(150)
    return successful(vendor, method)
  }
  const start = performance.now()
  const { report } = await run({ pollTimeoutMs: 20 })
  expect(report.vendors[1]?.task.status).toBe('error')
  expect(performance.now() - start).toBeLessThan(120)
})

test('tests that HTTP 500 is a vendor error', async () => {
  handler = () => new Response('vendor unavailable', { status: 500 })
  const { report } = await run()
  expect(report.vendors.every((v) => v.message.includes('HTTP_500'))).toBe(true)
  expect(report.exitCode).toBe(1)
})

test('tests that a hanging balance request times out without a paid task', async () => {
  handler = async () => {
    await Bun.sleep(150)
    return { errorId: 0, balance: 1 }
  }
  const { report } = await run({ requestTimeoutMs: 10 })
  expect(
    report.vendors.every(
      (v) => v.status === 'error' && v.message.includes('REQUEST_TIMEOUT'),
    ),
  ).toBe(true)
  expect(report.paidTasksCreated).toBe(0)
})

test.each([
  null,
  {},
  { errorId: 0, balance: -1 },
  { errorId: 0, balance: '1.25' },
])(
  'tests that invalid balance replies cannot authorize spending: %j',
  async (reply) => {
    handler = () => reply
    const { report } = await run()
    expect(report.exitCode).toBe(1)
    expect(report.paidTasksCreated).toBe(0)
  },
)

test('tests that task errors fail readiness without retrying paid work', async () => {
  handler = (vendor, method) =>
    method === 'getBalance'
      ? successful(vendor, method)
      : {
          errorId: 1,
          errorCode: 'ERROR_TASK',
          errorDescription: 'cannot classify',
        }
  const { report } = await run()
  expect(report.paidTasksCreated).toBe(2)
  expect(
    report.vendors.every(
      (v) => v.task.status === 'error' && v.task.errorCode === 'ERROR_TASK',
    ),
  ).toBe(true)
  expect(report.paidSolvingReady).toEqual({
    capsolver: false,
    '2captcha': false,
  })
  expect(report.exitCode).toBe(1)
})

test('tests that malformed solutions are errors rather than successful empty answers', async () => {
  handler = (vendor, method) =>
    method === 'createTask' && vendor === 'capsolver'
      ? { errorId: 0, status: 'ready', solution: {} }
      : successful(vendor, method)
  const { report } = await run()
  expect(report.vendors[0]?.task.status).toBe('error')
  expect(report.paidSolvingReady.capsolver).toBe(false)
  expect(report.exitCode).toBe(1)
})

test('tests that balance-only skips funded tasks and retains exit one', async () => {
  const result = await cli(['--balance-only'])
  expect(result.exitCode).toBe(1)
  expect(result.stdout).toContain('balance=$1.25')
  expect(result.stdout).toContain('task=skipped')
  expect(result.stdout).toContain('paid tasks created: 0')
  expect(result.stdout).toContain(
    'paid solving ready: capsolver=no 2captcha=no',
  )
  expect(calls).toHaveLength(2)
})

test('tests that JSON is one report with the same diagnostic facts', async () => {
  handler = () => ({ errorId: 0, balance: 0 })
  const result = await cli(['--json'])
  expect(result.exitCode).toBe(1)
  expect(result.stderr).toBe('')
  const report = JSON.parse(result.stdout)
  expect(Number.isNaN(Date.parse(report.timestamp))).toBe(false)
  expect(report.paidTasksCreated).toBe(0)
  expect(report.vendors.map((v: { balance: number }) => v.balance)).toEqual([
    0, 0,
  ])
})

test('tests that the real CLI exits zero only when both tasks succeed', async () => {
  const result = await cli()
  expect(result.exitCode).toBe(0)
  expect(result.stdout).toMatch(/^captcha provider check /)
  expect(result.stdout).toMatch(/task=ok \d+ms tiles=\[0,4,7\]/)
  expect(result.stdout).toContain('paid tasks created: 2')
  expect(result.stdout).toContain(
    'paid solving ready: capsolver=yes 2captcha=yes',
  )
  expect(result.stderr).toBe('')
})

test('tests that unknown flags exit two without calling vendors', async () => {
  const result = await cli(['--unknown'])
  expect(result.exitCode).toBe(2)
  expect(result.stderr).toContain('Usage:')
  expect(calls).toHaveLength(0)
})

test('tests that an empty argument followed by a bad flag exits two without vendor calls', async () => {
  const result = await cli(['', '--bogus'])
  expect(result.exitCode).toBe(2)
  expect(result.stderr).toContain('Usage:')
  expect(calls).toHaveLength(0)
})

test.each([{ args: [] }, { args: ['--json'] }])(
  'tests that vendor echo redaction protects every CLI stream: %j',
  async ({ args }) => {
    handler = () => ({
      errorId: 1,
      errorCode: 'ERROR_KEY_DENIED_ACCESS',
      errorDescription: `denied ${key} ${key}`,
    })
    const result = await cli(args)
    expect(result.exitCode).toBe(1)
    expect(result.stdout).toContain('denied *** ***')
    expect(result.stdout).not.toContain(key)
    expect(result.stderr).not.toContain(key)
  },
)

test('tests that usage errors also redact key values', async () => {
  const result = await cli([`--${key}`])
  expect(result.exitCode).toBe(2)
  expect(result.stderr).toContain('***')
  expect(result.stdout + result.stderr).not.toContain(key)
})

test('tests that JSON redacts escaped keys and both vendors keys', async () => {
  const otherKey = 'other-unit-test-key-"quoted"-\\-123'
  const otherPath = join(directory, 'other-key')
  await writeFile(otherPath, otherKey)
  env.BROWSEROS_2CAPTCHA_API_KEY_FILE = otherPath
  handler = () => ({
    errorId: 1,
    errorCode: 'ERROR_ECHO',
    errorDescription: `${key} and ${otherKey}`,
  })
  const result = await cli(['--json'])
  expect(result.exitCode).toBe(1)
  const report = JSON.parse(result.stdout)
  expect(
    report.vendors.every(
      (v: { message: string }) => v.message === 'ERROR_ECHO: *** and ***',
    ),
  ).toBe(true)
  expect(result.stdout + result.stderr).not.toContain(key)
  expect(result.stdout + result.stderr).not.toContain('quoted')
})

test('tests that a file error containing the other vendor key is redacted', async () => {
  env.BROWSEROS_CAPSOLVER_API_KEY_FILE = join(directory, key, 'missing')
  handler = () => ({ errorId: 0, balance: 0 })
  const result = await cli()
  expect(result.exitCode).toBe(1)
  expect(result.stdout).toContain('***')
  expect(result.stdout + result.stderr).not.toContain(key)
})

test('tests that malformed JSON cannot authorize paid tasks', async () => {
  handler = () => new Response('not-json')
  const { report } = await run()
  expect(report.vendors.every((v) => v.status === 'error')).toBe(true)
  expect(report.paidTasksCreated).toBe(0)
})

test('tests that 2Captcha rejects an invalid task ID without polling', async () => {
  handler = (vendor, method) =>
    vendor === '2captcha' && method === 'createTask'
      ? { errorId: 0 }
      : successful(vendor, method)
  const { report } = await run()
  expect(report.vendors[1]?.task.errorCode).toBe('INVALID_RESPONSE')
  expect(calls.some((c) => c.method === 'getTaskResult')).toBe(false)
})

test('tests that a failed 2Captcha poll preserves its vendor error', async () => {
  handler = (vendor, method) =>
    method === 'getTaskResult'
      ? {
          errorId: 12,
          errorCode: 'ERROR_POLL',
          errorDescription: 'poll failed',
        }
      : successful(vendor, method)
  const { report } = await run()
  expect(report.vendors[1]?.task.errorCode).toBe('ERROR_POLL')
  expect(report.vendors[1]?.message).toBe('ERROR_POLL: poll failed')
})

test('tests that prefixed settings work with irrelevant unprefixed variables', async () => {
  for (const vendor of ['CAPSOLVER', '2CAPTCHA']) {
    for (const suffix of ['API_KEY_FILE', 'API_URL']) {
      env[`${vendor}_${suffix}`] = 'invalid-alias'
    }
  }
  const { report } = await run()
  expect(report.exitCode).toBe(0)
  expect(report.paidTasksCreated).toBe(2)
})

test('tests that HTTP errors preserve structured vendor error details', async () => {
  handler = () =>
    Response.json(
      {
        errorId: 1,
        errorCode: 'ERROR_KEY_DENIED_ACCESS',
        errorDescription: `authorization invalid ${key}`,
      },
      { status: 401 },
    )
  const result = await cli(['--json'])
  const report = JSON.parse(result.stdout)
  expect(
    report.vendors.every(
      (v: { message: string }) =>
        v.message === 'ERROR_KEY_DENIED_ACCESS: authorization invalid ***',
    ),
  ).toBe(true)
  expect(result.exitCode).toBe(1)
  expect(result.stdout + result.stderr).not.toContain(key)
})

test('tests that a missing optional 2Captcha key skips its checks and exits zero', async () => {
  env.BROWSEROS_2CAPTCHA_API_KEY_FILE = join(directory, 'missing-optional-key')
  const result = await cli()
  expect(result.exitCode).toBe(0)
  expect(result.stdout).toContain(
    '2captcha    skipped balance=n/a    task=skipped  no key file; unverified: no key yet',
  )
  expect(result.stdout).toContain('paid tasks created: 1')
  expect(result.stdout).toContain(
    'paid solving ready: capsolver=yes 2captcha=no',
  )
  expect(result.stderr).toBe('')
  expect(calls.some(({ vendor }) => vendor === '2captcha')).toBe(false)
})

test('tests that a configured zero 2Captcha balance fails readiness', async () => {
  handler = (vendor, method) =>
    vendor === '2captcha' && method === 'getBalance'
      ? { errorId: 0, balance: 0 }
      : successful(vendor, method)
  const { report } = await run()
  expect(report.exitCode).toBe(1)
  expect(report.vendors[0]?.task.status).toBe('ok')
  expect(report.vendors[1]).toMatchObject({
    vendor: '2captcha',
    status: 'ok',
    balance: 0,
    task: { status: 'skipped' },
  })
  expect(report.paidTasksCreated).toBe(1)
  expect(report.paidSolvingReady).toEqual({
    capsolver: true,
    '2captcha': false,
  })
})

test('tests that a configured 2Captcha vendor error fails the check', async () => {
  handler = (vendor, method) =>
    vendor === '2captcha'
      ? {
          errorId: 1,
          errorCode: 'ERROR_KEY_DOES_NOT_EXIST',
          errorDescription: 'invalid key',
        }
      : successful(vendor, method)
  const { report } = await run()
  expect(report.exitCode).toBe(1)
  expect(report.vendors[0]?.task.status).toBe('ok')
  expect(report.vendors[1]).toMatchObject({
    status: 'error',
    balance: null,
    message: 'ERROR_KEY_DOES_NOT_EXIST: invalid key',
  })
  expect(report.paidTasksCreated).toBe(1)
})

test('tests that an unreadable optional key is an error rather than a skip', async () => {
  env.BROWSEROS_2CAPTCHA_API_KEY_FILE = directory
  const { report } = await run()
  expect(report.vendors[1]?.status).toBe('error')
  expect(report.exitCode).toBe(1)
  expect(calls.some(({ vendor }) => vendor === '2captcha')).toBe(false)
})

test.each([
  {},
  { click: [0] },
  { click: [10] },
  { click: ['1'] },
  { click: [1.5] },
])(
  'tests that malformed 2Captcha grid answers fail readiness: %j',
  async (solution) => {
    handler = (vendor, method) =>
      vendor === '2captcha' && method === 'getTaskResult'
        ? { errorId: 0, status: 'ready', solution }
        : successful(vendor, method)
    const { report } = await run()
    expect(report.vendors[1]?.task.errorCode).toBe('INVALID_RESPONSE')
    expect(report.paidSolvingReady['2captcha']).toBe(false)
    expect(report.exitCode).toBe(1)
  },
)
