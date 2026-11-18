import { mkdir, mkdtemp, open, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { findFreePort } from '../../../../../contracts/claw-mcp/tests/browser'

const AGENT_ROOT = resolve(import.meta.dir, '../../../../..')

/** neo-server's `browseros-claw-sdk`: `CLAW_SDK_BINARY`, else Cargo's debug build. */
export function sdkBinary(env: NodeJS.ProcessEnv = process.env): string {
  return resolve(
    AGENT_ROOT,
    env.CLAW_SDK_BINARY ??
      join(env.CARGO_TARGET_DIR ?? 'target', 'debug/browseros-claw-sdk'),
  )
}

/** Builds the server unless `CLAW_SDK_BINARY` names a prebuilt one. */
export function prepareSdkServer(
  env: NodeJS.ProcessEnv = process.env,
  build = buildSdkServer,
): void {
  if (!env.CLAW_SDK_BINARY) build()
}

/** Built before the browser starts, so no timeout absorbs a cold Cargo build. */
export function buildSdkServer(spawnSync = Bun.spawnSync): void {
  const build = spawnSync({
    cmd: [
      'cargo',
      'build',
      '--locked',
      '-p',
      'neo-server',
      '--bin',
      'browseros-claw-sdk',
    ],
    cwd: AGENT_ROOT,
    stdout: 'inherit',
    stderr: 'inherit',
  })
  if (build.exitCode !== 0)
    throw new Error(`cargo build -p neo-server failed (${build.exitCode})`)
}

/**
 * Starts the SDK server against a browser's CDP port, as the browser would: a launch file with
 * its ports, in a private state directory, so its trace store never touches the owner's.
 */
export async function startSdkServer(
  cdpPort: number,
  options: {
    env: Record<string, string | undefined>
    logPath: string
    binary?: string
    readyTimeoutMs?: number
  },
) {
  const root = await mkdtemp(join(tmpdir(), 'captcha-sdk-'))
  try {
    const port = await findFreePort()
    await mkdir(join(root, 'resources'))
    await mkdir(join(root, 'home'))
    const config = join(root, 'launch.json')
    await writeFile(
      config,
      JSON.stringify({
        ports: { server: port, cdp: cdpPort },
        directories: { resources: 'resources' },
      }),
    )
    const env: Record<string, string> = {}
    for (const [name, value] of Object.entries({
      ...process.env,
      ...options.env,
      HOME: join(root, 'home'),
      BROWSERCLAW_DIR: join(root, 'state'),
      BROWSERCLAW_HOSTED_MODE: '1',
    }))
      if (value !== undefined && !name.startsWith('CLAW_HOST_'))
        env[name] = value
    const log = await open(options.logPath, 'w')
    let child: Bun.Subprocess
    try {
      child = Bun.spawn({
        cmd: [options.binary ?? sdkBinary(), '--config', config],
        cwd: root,
        env,
        stdout: log.fd,
        stderr: log.fd,
      })
    } finally {
      await log.close()
    }
    const baseUrl = `http://127.0.0.1:${port}`
    const stop = async () => {
      if (child.exitCode === null) {
        child.kill()
        const force = setTimeout(() => child.kill(9), 6000)
        await child.exited
        clearTimeout(force)
      }
      await rm(root, { recursive: true, force: true })
    }
    const deadline = Date.now() + (options.readyTimeoutMs ?? 60_000)
    while (true) {
      if (child.exitCode !== null) {
        await stop()
        throw new Error(
          `browseros-claw-sdk exited with ${child.exitCode}; see ${options.logPath}`,
        )
      }
      const ready = await fetch(`${baseUrl}/system/ready`, {
        signal: AbortSignal.timeout(1000),
      }).then(
        (response) => response.ok,
        () => false,
      )
      if (ready) return { baseUrl, stop }
      if (Date.now() > deadline) {
        await stop()
        throw new Error(
          `browseros-claw-sdk was not ready in time; see ${options.logPath}`,
        )
      }
      await Bun.sleep(250)
    }
  } catch (error) {
    await rm(root, { recursive: true, force: true })
    throw error
  }
}
