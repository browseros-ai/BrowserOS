import { constants } from 'node:fs'
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { McpSession } from '../../../../../contracts/claw-mcp/tests/mcp-client'
import { DEFAULT_KEY_DIR } from '../e2e/keys'

const root = resolve(import.meta.dir, '../../../../..')
const BUILD_SCRIPT = 'crates/neo-sdk/neo-server/docker/build.sh'
/** Where the image's README asks a host to mount keys: outside /home/neo, which Neo re-owns. */
const KEY_DIR = '/run/neo-keys'
const PROVIDERS = [
  { variable: 'BROWSEROS_CAPSOLVER_API_KEY_FILE', file: 'capsolver-api-key' },
  { variable: 'BROWSEROS_2CAPTCHA_API_KEY_FILE', file: '2captcha-api-key' },
]
export type Keys = 'none' | 'placeholder' | 'real'
export const DEFAULT_REAL_PAID_TASK_LIMIT = 12

export class CommandError extends Error {
  constructor(
    readonly exitCode: number,
    message: string,
  ) {
    super(message)
  }
}

function defaults() {
  return {
    keys: 'placeholder' as Keys,
    image: undefined as string | undefined,
    out: resolve(root, '../../.llm/captcha-docker-results'),
    paidTaskLimit: undefined as number | undefined,
    keep: false,
    clean: false,
    help: false,
    passthrough: [] as string[],
  }
}
type Options = ReturnType<typeof defaults>
const VALUE_FLAGS: Record<string, (options: Options, value: string) => void> = {
  '--keys': (options, value) => {
    options.keys = value as Keys
  },
  '--image': (options, value) => {
    options.image = value
  },
  '--out': (options, value) => {
    options.out = resolve(value)
  },
  '--paid-task-limit': (options, value) => {
    const limit = Number(value)
    if (!Number.isInteger(limit) || limit < 0)
      throw new Error('--paid-task-limit must be a whole number')
    options.paidTaskLimit = limit
  },
}
const SWITCHES: Record<string, 'keep' | 'clean' | 'help'> = {
  '--keep': 'keep',
  '--clean': 'clean',
  '--help': 'help',
}

export function parseArgs(argv: string[]) {
  const options = defaults()
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i] ?? ''
    const set = VALUE_FLAGS[arg]
    if (set) {
      const value = argv[++i]
      if (!value || value.startsWith('--'))
        throw new Error(`${arg} requires a value`)
      set(options, value)
    } else if (SWITCHES[arg]) options[SWITCHES[arg]] = true
    else if (arg && arg !== '--') options.passthrough.push(arg)
  }
  if (!['none', 'placeholder', 'real'].includes(options.keys))
    throw new Error('Invalid --keys mode')
  if (options.keep && options.clean)
    throw new Error('--keep and --clean conflict')
  if (options.clean && options.image)
    throw new Error('--clean removes only an image this run built, not --image')
  if (
    options.passthrough.some((arg) =>
      /^--(mcp-url|server|fixture-bind|fixture-host|fixture-frame-host|fixture-origin)(=|$)/.test(
        arg,
      ),
    )
  )
    throw new Error('The harness owns the server and the fixture addresses')
  options.paidTaskLimit ??=
    options.keys === 'real' ? DEFAULT_REAL_PAID_TASK_LIMIT : undefined
  return options
}

/**
 * The container's key mounts. Each provider's variable is always set, blank without a key, so a
 * key baked into the image is never read. Key contents are never read or copied.
 */
export async function keyMounts(
  mode: Keys,
  dir: string,
  env: NodeJS.ProcessEnv,
  home = homedir(),
) {
  const args: string[] = []
  for (const { variable, file } of PROVIDERS) {
    let source: string | undefined
    if (mode === 'placeholder' && variable === PROVIDERS[0]?.variable) {
      source = join(dir, file)
      await mkdir(dir, { recursive: true })
      await writeFile(source, 'captcha-e2e-placeholder\n', { mode: 0o444 })
    } else if (mode === 'real') {
      const configured = env[variable]
      const fallback = join(home, DEFAULT_KEY_DIR, file)
      if (configured)
        source = resolve(
          configured.startsWith('~/')
            ? join(home, configured.slice(2))
            : configured,
        )
      else if (
        await access(fallback).then(
          () => true,
          () => false,
        )
      )
        source = fallback
    }
    if (!source) {
      args.push(`--env=${variable}=`)
      continue
    }
    if (source.includes(','))
      throw new Error('Key mount paths cannot contain commas')
    await access(source, constants.R_OK)
    if (!(await stat(source)).isFile())
      throw new Error('Key path must be a regular file')
    args.push(
      `--mount=type=bind,src=${source},dst=${KEY_DIR}/${file},readonly`,
      `--env=${variable}=${KEY_DIR}/${file}`,
    )
  }
  if (mode === 'real' && !args.some((arg) => arg.startsWith('--mount')))
    throw new Error(
      `--keys real found no CapSolver or 2Captcha key file (variables or ~/${DEFAULT_KEY_DIR})`,
    )
  return args
}

export interface Runtime {
  env: NodeJS.ProcessEnv
  home: string
  /** Runs a command; `stream` shows its output live instead of returning it. */
  exec: (
    args: string[],
    env?: NodeJS.ProcessEnv,
    signal?: AbortSignal,
    stream?: boolean,
  ) => Promise<string>
  tools: (origin: string) => Promise<string[]>
}

async function command(
  args: string[],
  env: NodeJS.ProcessEnv = process.env,
  signal?: AbortSignal,
  stream = false,
) {
  signal?.throwIfAborted()
  const child = Bun.spawn(args, {
    cwd: root,
    env,
    stdout: stream ? 'inherit' : 'pipe',
    stderr: 'inherit',
  })
  const stop = () => child.kill('SIGTERM')
  signal?.addEventListener('abort', stop, { once: true })
  try {
    const output = stream ? '' : await new Response(child.stdout).text()
    const code = await child.exited
    if (code) throw new CommandError(code, `${args[0]} ${args[1]} failed`)
    signal?.throwIfAborted()
    return output.trim()
  } finally {
    signal?.removeEventListener('abort', stop)
  }
}

interface Provenance {
  revision?: string
  dirty?: boolean
  server?: { binary?: string; path?: string; sha256?: string }
}

export async function runDockerE2e(
  argv: string[],
  overrides: Partial<Runtime> = {},
) {
  const options = parseArgs(argv)
  if (options.help) {
    console.log(
      'Usage: bun run test:captcha-docker-e2e -- [--keys placeholder|real|none] [--image TAG] [--paid-task-limit N] [--out DIR] [--keep|--clean] [runner flags]',
    )
    return
  }
  const { exec, env, tools, home } = {
    exec: command,
    env: process.env,
    home: homedir(),
    tools: async (origin: string) => {
      const session = await McpSession.connect(origin, 'captcha-docker-proof')
      try {
        return (await session.listTools()).map((tool) => tool.name)
      } finally {
        await session.close()
      }
    },
    ...overrides,
  }
  await mkdir(options.out, { recursive: true })
  const temporary = await mkdtemp(join(tmpdir(), 'captcha-docker-e2e-'))
  const run = crypto.randomUUID()
  const name = `captcha-e2e-${run}`
  let image = options.image
  const timings: Record<string, number> = {}
  const proof: Record<string, unknown> = {
    keys: options.keys,
    paidTaskLimit: options.paidTaskLimit ?? null,
    container: name,
    network: name,
  }
  let built = false
  let networked = false
  let started = false
  let succeeded = false
  let cleanupError: unknown
  const timed = async <T>(label: string, action: () => Promise<T>) => {
    const start = performance.now()
    console.log(`[captcha-docker] ${label}`)
    try {
      return await action()
    } finally {
      timings[label] = Math.round(performance.now() - start)
    }
  }
  const controller = new AbortController()
  const execute = (args: string[], stream = false) =>
    exec(args, env, controller.signal, stream)
  const docker = (...args: string[]) => execute(['docker', ...args])
  const cleanupDocker = (...args: string[]) => exec(['docker', ...args], env)
  const writeProof = (extra: Record<string, unknown> = {}) =>
    writeFile(
      join(options.out, 'proof.json'),
      JSON.stringify({ ...proof, timings, ...extra }, null, 2),
    )
  /** Hashes each running server as Neo's user: root cannot read another uid's /proc/PID/exe. */
  const identity = async (path: string, expected: string) => {
    const actual = await docker(
      'exec',
      '--user=neo',
      name,
      'sh',
      '-ec',
      `found=0; for p in /proc/[0-9]*/exe; do if [ "$(readlink "$p" 2>/dev/null || true)" = '${path}' ]; then sha256sum "$p"; found=1; fi; done; test "$found" = 1`,
    )
    const lines = actual.trim().split('\n').filter(Boolean)
    if (
      !lines.length ||
      lines.some((line) => line.split(/\s+/)[0] !== expected)
    )
      throw new Error(
        "The running server is not the image's browseros-claw-sdk (possible OTA replacement)",
      )
    return actual
  }
  const interrupted = () => controller.abort(new Error('Run interrupted'))
  process.on('SIGINT', interrupted)
  process.on('SIGTERM', interrupted)
  try {
    const mounts = await keyMounts(
      options.keys,
      join(temporary, 'keys'),
      env,
      home,
    )
    if (!image) {
      await timed('image build', async () => {
        const output = await execute([
          'bash',
          BUILD_SCRIPT,
          '--tag',
          `neo-captcha-e2e:${run.slice(0, 8)}`,
        ])
        image = output.match(/^TAG=(\S+)$/m)?.[1]
        if (!image) throw new Error('build.sh printed no TAG=')
        built = true
      })
    }
    proof.image = image
    // The image's own facts, read without starting Neo: which server it installed, and from what.
    const facts = await docker(
      'run',
      '--rm',
      '--network=none',
      '--entrypoint=sh',
      image as string,
      '-ec',
      'neo-server-info kind; cat /usr/share/neo-sdk/provenance.json',
    )
    const [kind, ...json] = facts.split('\n')
    const provenance = JSON.parse(json.join('\n')) as Provenance
    proof.kind = kind
    proof.provenance = provenance
    const serverPath = provenance.server?.path ?? ''
    const sha256 = provenance.server?.sha256 ?? ''
    if (
      kind?.trim() !== 'claw-sdk' ||
      provenance.server?.binary !== 'browseros-claw-sdk' ||
      !/^\/(usr\/lib|opt)\/[\w./-]+$/.test(serverPath) ||
      !/^[0-9a-f]{64}$/.test(sha256)
    )
      throw new Error(
        `${image} does not run neo-server's browseros-claw-sdk (kind ${kind || 'unknown'})`,
      )
    if (built) {
      const head = await execute(['git', 'rev-parse', 'HEAD'])
      if (provenance.revision !== head)
        throw new Error(
          `${image} was built from ${provenance.revision}, not ${head}`,
        )
    }
    const data = join(temporary, 'data')
    await mkdir(data)
    await chmod(temporary, 0o755)
    // Its own network: NEO_MCP_REMOTE=true admits every peer that can route to the container.
    await docker('network', 'create', name)
    networked = true
    await timed('container health', async () => {
      // Name is owned before launch, so cleanup also covers a partially failed run.
      started = true
      await docker(
        'run',
        '--detach',
        `--name=${name}`,
        `--network=${name}`,
        '--shm-size=1g',
        '--add-host=host.docker.internal:host-gateway',
        '--add-host=captcha-frame.test:host-gateway',
        '--publish=127.0.0.1::9010',
        `--mount=type=bind,src=${data},dst=/data`,
        ...mounts,
        '--env=NEO_MCP_REMOTE=true',
        `--env=PUID=${process.getuid?.() ?? 1000}`,
        `--env=PGID=${process.getgid?.() ?? 1000}`,
        ...(options.paidTaskLimit === undefined
          ? []
          : [
              `--env=BROWSEROS_CAPTCHA_PAID_TASK_LIMIT=${options.paidTaskLimit}`,
            ]),
        image as string,
      )
      const deadline = Date.now() + 180_000
      while (Date.now() < deadline) {
        const [info] = JSON.parse(await docker('inspect', name))
        if (info.State.Health?.Status === 'healthy') {
          const port = info.NetworkSettings.Ports['9010/tcp'][0].HostPort
          proof.origin = `http://127.0.0.1:${port}`
          return
        }
        if (
          info.State.Status !== 'running' ||
          info.State.Health?.Status === 'unhealthy'
        )
          throw new Error('Container failed its healthcheck')
        await Bun.sleep(1000)
      }
      throw new Error('Container healthcheck timed out')
    })
    proof.runningServer = await timed('server proof', () =>
      identity(serverPath, sha256),
    )
    proof.tools = await tools(String(proof.origin))
    await writeProof()
    await timed('page tests', () =>
      execute(
        [
          'bun',
          'run',
          'test:captcha-e2e',
          '--',
          '--mcp-url',
          `${proof.origin}/mcp`,
          '--fixture-bind',
          '0.0.0.0',
          '--fixture-host',
          'host.docker.internal',
          '--fixture-frame-host',
          'captcha-frame.test',
          '--pages',
          'all',
          '--keys',
          options.keys,
          ...options.passthrough,
          '--out',
          options.out,
        ],
        true,
      ),
    )
    proof.runningServerAfter = await identity(serverPath, sha256)
    succeeded = true
  } finally {
    process.off('SIGINT', interrupted)
    process.off('SIGTERM', interrupted)
    if (started && options.keep && !controller.signal.aborted) {
      console.log(
        `Kept: ${proof.origin ?? name}\nCleanup: docker rm -f ${name}; docker network rm ${name}; rm -rf '${temporary}'`,
      )
      proof.kept = { container: name, network: name, directory: temporary }
    } else {
      try {
        if (started) await cleanupDocker('rm', '-f', name)
        if (networked) await cleanupDocker('network', 'rm', name)
      } catch (error) {
        cleanupError = error
        proof.kept = { container: name, network: name, directory: temporary }
      }
      if (!cleanupError) await rm(temporary, { recursive: true, force: true })
    }
    if (options.clean && built && !cleanupError) {
      try {
        await cleanupDocker('image', 'rm', image as string)
      } catch (error) {
        cleanupError = error
      }
    }
    await writeProof({ succeeded: succeeded && !cleanupError })
  }
  if (cleanupError)
    throw new Error(
      `Docker cleanup failed; inspect ${join(options.out, 'proof.json')}`,
    )
}

if (import.meta.main) {
  runDockerE2e(process.argv.slice(2)).catch((error) => {
    console.error(
      `[captcha-docker] ${error instanceof Error ? error.message : 'Run failed'}`,
    )
    // The runner's own code passes through: 2 is a setup blocker, such as a missing key.
    process.exitCode =
      error instanceof CommandError && error.exitCode === 2 ? 2 : 1
  })
}
