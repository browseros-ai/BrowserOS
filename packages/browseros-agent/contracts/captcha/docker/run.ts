import { constants } from 'node:fs'
import {
  access,
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import { isAbsolute, join, resolve } from 'node:path'
import { McpSession } from '../../claw-mcp/tests/mcp-client'

const root = resolve(import.meta.dir, '../../..')
const binaryName = 'browseros-claw-server-rs'
const rustTarget = 'aarch64-unknown-linux-gnu'
export type Keys = 'none' | 'placeholder' | 'real'

export function parseArgs(argv: string[]) {
  const options = {
    keys: 'placeholder' as Keys,
    version: '0.51.1.4',
    builder: 'auto',
    out: resolve(root, '../../.llm/captcha-docker-results'),
    keep: false,
    clean: false,
    rebuild: false,
    help: false,
    passthrough: [] as string[],
  }
  const valueFlags: Record<string, 'keys' | 'version' | 'builder' | 'out'> = {
    '--keys': 'keys',
    '--neo-version': 'version',
    '--builder': 'builder',
    '--out': 'out',
  }
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    if (arg === '--') continue
    const field = valueFlags[arg ?? '']
    if (field) {
      const value = argv[++i]
      if (!value || value.startsWith('--'))
        throw new Error(`${arg} requires a value`)
      Object.assign(options, {
        [field]: field === 'out' ? resolve(value) : value,
      })
    } else if (arg === '--keep') options.keep = true
    else if (arg === '--clean') options.clean = true
    else if (arg === '--rebuild') options.rebuild = true
    else if (arg === '--help') options.help = true
    else if (arg) options.passthrough.push(arg)
  }
  if (!['none', 'placeholder', 'real'].includes(options.keys))
    throw new Error('Invalid --keys mode')
  if (!['auto', 'zig', 'container'].includes(options.builder))
    throw new Error('Invalid --builder')
  if (!/^\d+(\.\d+){1,3}$/.test(options.version))
    throw new Error('Invalid --neo-version')
  if (options.keep && options.clean)
    throw new Error('--keep and --clean conflict')
  if (
    options.passthrough.some((arg) =>
      /^--(mcp-url|fixture-bind|fixture-host)(=|$)/.test(arg),
    )
  ) {
    throw new Error('The harness owns MCP and fixture addresses')
  }
  return options
}

export async function keyMounts(
  mode: Keys,
  dir: string,
  env: NodeJS.ProcessEnv,
) {
  if (mode === 'none') return []
  const args: string[] = []
  const primary = 'BROWSEROS_CAPSOLVER_API_KEY_FILE'
  const files: Record<string, string> = {
    [primary]:
      mode === 'real'
        ? env[primary] ||
          join(homedir(), '.config/browseros/captcha/capsolver-api-key')
        : join(dir, 'capsolver-api-key'),
  }
  if (mode === 'real') {
    for (const [variable, value] of Object.entries(env)) {
      if (
        variable !== primary &&
        /^BROWSEROS_\w+_API_KEY_FILE$/.test(variable) &&
        value
      )
        files[variable] = value
    }
  }
  for (const [variable, value] of Object.entries(files)) {
    const source = resolve(value)
    const destination =
      variable === primary ? '/run/captcha/capsolver-api-key' : value
    if (!isAbsolute(destination))
      throw new Error('Key-file paths must be absolute')
    if (source.includes(',') || destination.includes(','))
      throw new Error('Key mount paths cannot contain commas')
    if (mode === 'placeholder') {
      await writeFile(source, 'captcha-e2e-placeholder\n', { mode: 0o444 })
    }
    await access(source, constants.R_OK)
    if (!(await stat(source)).isFile())
      throw new Error('Key path must be a regular file')
    args.push(
      `--mount=type=bind,src=${source},dst=${destination},readonly`,
      `--env=${variable}=${destination}`,
    )
  }
  return args
}

export interface Runtime {
  env: NodeJS.ProcessEnv
  exec: (
    args: string[],
    env?: NodeJS.ProcessEnv,
    signal?: AbortSignal,
  ) => Promise<string>
  tools: (origin: string) => Promise<string[]>
}

async function command(
  args: string[],
  env: NodeJS.ProcessEnv = process.env,
  signal?: AbortSignal,
) {
  signal?.throwIfAborted()
  const child = Bun.spawn(args, {
    cwd: root,
    env,
    stdout: 'pipe',
    stderr: 'inherit',
  })
  const stop = () => child.kill('SIGTERM')
  signal?.addEventListener('abort', stop, { once: true })
  try {
    const output = await new Response(child.stdout).text()
    if (await child.exited) throw new Error(`${args[0]} ${args[1]} failed`)
    signal?.throwIfAborted()
    return output.trim()
  } finally {
    signal?.removeEventListener('abort', stop)
  }
}

export async function runDockerE2e(
  argv: string[],
  overrides: Partial<Runtime> = {},
) {
  const options = parseArgs(argv)
  if (options.help) {
    console.log(
      'Usage: bun run test:captcha-docker-e2e -- [--keys placeholder|real|none] [--neo-version VERSION] [--builder auto|zig|container] [--out DIR] [--rebuild] [--keep|--clean] [runner flags]',
    )
    return
  }
  const { exec, env, tools } = {
    exec: command,
    env: process.env,
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
  const targetDir = resolve(
    env.CARGO_TARGET_DIR ?? '/Volumes/extssd/cargo-target/captcha-docker-e2e',
  )
  const base = `neo-captcha-e2e-base:${options.version}`
  const image = `neo-captcha-e2e:${options.version}-${crypto.randomUUID().slice(0, 8)}`
  const name = `captcha-e2e-${crypto.randomUUID()}`
  const timings: Record<string, number> = {}
  const proof: Record<string, unknown> = {
    version: options.version,
    base,
    image,
    targetDir,
  }
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
  const execute = (args: string[], buildEnv = env) =>
    exec(args, buildEnv, controller.signal)
  const docker = (...args: string[]) => execute(['docker', ...args])
  const cleanupDocker = (...args: string[]) => exec(['docker', ...args], env)
  const identity = async (expected: string) => {
    // Hash the executable inode, not merely the file we overlaid on disk.
    const actual = await docker(
      'exec',
      '--user=neo',
      name,
      'sh',
      '-ec',
      'found=0; for p in /proc/[0-9]*/exe; do case "$(readlink "$p" 2>/dev/null || true)" in */browseros-claw-server|*/browseros-claw-server-rs) sha256sum "$p"; found=1;; esac; done; test "$found" = 1',
    )
    if (
      !actual.trim() ||
      actual
        .trim()
        .split('\n')
        .some((line) => line.split(/\s+/)[0] !== expected)
    ) {
      throw new Error(
        'Running claw-server hash differs from build output (possible OTA replacement)',
      )
    }
    return actual
  }
  const interrupted = () => controller.abort(new Error('Run interrupted'))
  process.on('SIGINT', interrupted)
  process.on('SIGTERM', interrupted)
  try {
    const mounts = await keyMounts(options.keys, temporary, env)
    proof.diskBefore = await docker('system', 'df')
    await timed('base image', async () => {
      const cached = await docker('image', 'inspect', base).then(
        () => true,
        () => false,
      )
      if (options.rebuild || !cached)
        await docker(
          'build',
          '--platform=linux/arm64',
          `--build-arg=NEO_VERSION=${options.version}`,
          `--tag=${base}`,
          resolve(root, '../browseros/docker/neo'),
        )
    })
    await timed('server build', async () => {
      const cargoArgs = [
        '--release',
        '--locked',
        '--package=claw-server',
        `--bin=${binaryName}`,
      ]
      const zig =
        options.builder === 'zig' ||
        (options.builder === 'auto' && Bun.which('cargo-zigbuild'))
      if (zig)
        await execute(
          ['cargo', 'zigbuild', '--target', `${rustTarget}.2.17`, ...cargoArgs],
          { ...env, CARGO_TARGET_DIR: targetDir },
        )
      else {
        const registry = join(targetDir, 'registry')
        await mkdir(registry, { recursive: true })
        await docker(
          'run',
          '--rm',
          '--platform=linux/arm64',
          `--mount=type=bind,src=${root},dst=/src,readonly`,
          `--mount=type=bind,src=${targetDir},dst=/target`,
          `--mount=type=bind,src=${registry},dst=/usr/local/cargo/registry`,
          '--workdir=/src',
          '--env=CARGO_TARGET_DIR=/target',
          'rust:1.96-bookworm',
          'cargo',
          'build',
          '--target',
          rustTarget,
          ...cargoArgs,
        )
      }
    })
    const binary = join(targetDir, rustTarget, 'release', binaryName)
    const hash = new Bun.CryptoHasher('sha256')
      .update(await Bun.file(binary).arrayBuffer())
      .digest('hex')
    const serverPath = (
      await docker(
        'run',
        '--rm',
        '--entrypoint=sh',
        base,
        '-ec',
        'find /usr/lib /opt -type f -name browseros-claw-server',
      )
    ).trim()
    if (!/^\/(usr\/lib|opt)\/[\w./-]+$/.test(serverPath))
      throw new Error('Expected exactly one bundled claw-server')
    proof.serverPath = serverPath
    proof.sha256 = hash
    const context = join(temporary, 'overlay')
    await mkdir(context)
    await copyFile(binary, join(context, binaryName))
    await copyFile(
      join(import.meta.dir, 'Dockerfile.e2e'),
      join(context, 'Dockerfile'),
    )
    await timed('overlay image', () =>
      docker(
        'build',
        '--platform=linux/arm64',
        `--build-arg=BASE_IMAGE=${base}`,
        `--build-arg=SERVER_PATH=${serverPath}`,
        `--tag=${image}`,
        context,
      ),
    )
    proof.imageSize = await docker('image', 'ls', '--format={{.Size}}', image)
    const data = join(temporary, 'data')
    await mkdir(data)
    await chmod(temporary, 0o755)
    await timed('container health', async () => {
      // Name is owned before launch, so cleanup also covers a partially failed run.
      started = true
      await docker(
        'run',
        '--detach',
        `--name=${name}`,
        '--shm-size=1g',
        '--add-host=host.docker.internal:host-gateway',
        '--publish=127.0.0.1::9010',
        '--publish=127.0.0.1::6080',
        `--mount=type=bind,src=${data},dst=/data`,
        ...mounts,
        `--env=PUID=${process.getuid?.() ?? 1000}`,
        `--env=PGID=${process.getgid?.() ?? 1000}`,
        '--env=NEO_ARGS=--disable-browseros-server-updater',
        image,
      )
      const deadline = Date.now() + 180_000
      while (Date.now() < deadline) {
        const [info] = JSON.parse(await docker('inspect', name))
        if (info.State.Health?.Status === 'healthy') {
          const port = (number: string) =>
            info.NetworkSettings.Ports[`${number}/tcp`][0].HostPort
          proof.origin = `http://127.0.0.1:${port('9010')}`
          proof.viewer = `http://127.0.0.1:${port('6080')}`
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
    console.log(`noVNC: ${proof.viewer}`)
    proof.runningBinary = await timed('overlay proof', () => identity(hash))
    proof.tools = await tools(String(proof.origin))
    await writeFile(
      join(options.out, 'proof.json'),
      JSON.stringify({ ...proof, timings }, null, 2),
    )
    await timed('page tests', () =>
      execute([
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
        '--pages',
        'all',
        '--keys',
        options.keys,
        ...options.passthrough,
        '--out',
        options.out,
      ]),
    )
    proof.runningBinaryAfter = await identity(hash)
    succeeded = true
  } finally {
    process.off('SIGINT', interrupted)
    process.off('SIGTERM', interrupted)
    if (started && options.keep && !controller.signal.aborted) {
      // Keep only the mounts, not the copied build binary.
      await rm(join(temporary, 'overlay'), { recursive: true, force: true })
      console.log(
        `noVNC: ${proof.viewer}\nCleanup: docker rm -f ${name}; rm -rf '${temporary}'`,
      )
      proof.kept = { container: name, directory: temporary }
    } else {
      if (started) {
        try {
          await cleanupDocker('rm', '-f', name)
        } catch (error) {
          cleanupError = error
          proof.kept = { container: name, directory: temporary }
        }
      }
      if (!cleanupError) await rm(temporary, { recursive: true, force: true })
    }
    if (options.clean && !cleanupError) {
      try {
        await cleanupDocker('image', 'rm', image, base)
      } catch (error) {
        cleanupError = error
      }
    }
    proof.diskAfter = await cleanupDocker('system', 'df').catch(
      () => 'unavailable',
    )
    await writeFile(
      join(options.out, 'proof.json'),
      JSON.stringify(
        { ...proof, timings, succeeded: succeeded && !cleanupError },
        null,
        2,
      ),
    )
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
    process.exitCode = 1
  })
}
