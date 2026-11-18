/** Owns the SDK sidecar and its authenticated host socket, borrowing the launcher's browser. */
import { constants } from 'node:fs'
import { access, mkdir, mkdtemp, open, rm, writeFile } from 'node:fs/promises'
import { type RequestOptions, request } from 'node:http'
import { isAbsolute, join } from 'node:path'
import { findFreePort } from '../../../../../contracts/claw-mcp/tests/browser'
import { jsonObject } from './json'
import type { BrowserRecord, ServerHandle } from './model'

interface ServerOptions {
  cdpPort: number
  binary: string
  browser: BrowserRecord
  logPath: string
}

interface ServerPaths {
  root: string
  tokenFile: string
  socketPath: string
}

/** A positive allowlist keeps provider keys, host sockets, and developer state out of the sidecar. */
export function serverEnvironment(
  paths: ServerPaths,
  browser: BrowserRecord,
  inherited: NodeJS.ProcessEnv,
): Record<string, string> {
  const env: Record<string, string> = {}
  for (const name of ['PATH', 'LANG', 'LC_ALL', 'LC_CTYPE', 'TZ']) {
    const value = inherited[name]
    if (value !== undefined) env[name] = value
  }
  const home = join(paths.root, 'home')
  return {
    ...env,
    HOME: home,
    XDG_CONFIG_HOME: join(home, '.config'),
    XDG_DATA_HOME: join(home, '.local/share'),
    XDG_CACHE_HOME: join(home, '.cache'),
    TMPDIR: join(paths.root, 'tmp'),
    BROWSERCLAW_DIR: join(paths.root, 'state'),
    BROWSERCLAW_HOSTED_MODE: '1',
    CLAW_PROFILE_DIR: browser.profile,
    CLAW_DOWNLOADS_DIR: browser.downloads,
    CLAW_HOST_TOKEN_FILE: paths.tokenFile,
    CLAW_HOST_SOCKET: paths.socketPath,
    // There is no Xvnc in these headless runs. Never contact an inherited display service.
    CLAW_XVNC_ADDR: '127.0.0.1:9',
  }
}

async function readiness(
  options: RequestOptions,
  settings: boolean,
): Promise<boolean> {
  try {
    return await new Promise<boolean>((resolve, reject) => {
      const call = request(options, (response) => {
        let body = ''
        response.setEncoding('utf8')
        response.on('data', (chunk: string) => {
          body += chunk
          if (body.length > 64 * 1024)
            call.destroy(new Error('Oversized readiness response'))
        })
        response.on('error', reject)
        response.on('end', () => {
          if (response.statusCode !== 200) return resolve(false)
          if (settings) {
            try {
              jsonObject(JSON.parse(body), 'host settings readiness')
            } catch (error) {
              reject(error)
              return
            }
          }
          resolve(true)
        })
      })
      const timer = setTimeout(
        () => call.destroy(new Error('Readiness timed out')),
        1000,
      )
      call.once('close', () => clearTimeout(timer))
      call.once('error', reject)
      call.end()
    })
  } catch {
    return false
  }
}

/** Startup is complete only when both the agent and this invocation's authenticated host are ready. */
export async function startServer(
  options: ServerOptions,
): Promise<ServerHandle> {
  if (
    !Number.isInteger(options.cdpPort) ||
    options.cdpPort < 1 ||
    options.cdpPort > 65535
  )
    throw new Error('The launcher must supply a valid CDP port')
  for (const path of [
    options.binary,
    options.browser.profile,
    options.browser.home,
    options.browser.downloads,
    options.logPath,
  ])
    if (!isAbsolute(path)) throw new Error('SDK process paths must be absolute')
  await access(options.binary, constants.X_OK)
  // macOS's TMPDIR may exceed the Unix socket limit. This directory is short and solely ours.
  const root = await mkdtemp('/tmp/sdk-desktop-')
  const paths = {
    root,
    tokenFile: join(root, 'token'),
    socketPath: join(root, 'host.sock'),
  }
  const token =
    crypto.randomUUID().replaceAll('-', '') +
    crypto.randomUUID().replaceAll('-', '')
  let child: Bun.Subprocess | undefined
  let stopped: Promise<void> | undefined
  let readyAt: number | undefined
  let agentPort: number | undefined
  const startedAt = Date.now()
  const stop = () =>
    (stopped ??= (async () => {
      let forced = false
      let cleaned = false
      let cleanupError: string | undefined
      try {
        if (child) {
          if (child.exitCode === null) {
            child.kill('SIGTERM')
            const force = setTimeout(() => {
              forced = true
              child?.kill('SIGKILL')
            }, 6000)
            try {
              await child.exited
            } finally {
              clearTimeout(force)
            }
          }
          if (forced || child.exitCode !== 0)
            cleanupError = `SDK shutdown ${forced ? 'required SIGKILL' : `exited ${child.exitCode}`}`
        }
        await rm(root, { recursive: true, force: true })
        cleaned = true
      } catch (error) {
        cleanupError = error instanceof Error ? error.message : String(error)
      } finally {
        await writeFile(
          `${options.logPath}.lifecycle.json`,
          `${JSON.stringify(
            {
              startedAt,
              readyAt,
              stoppedAt: Date.now(),
              pid: child?.pid,
              cdpPort: options.cdpPort,
              agentPort,
              root,
              socketPath: paths.socketPath,
              exitCode: child?.exitCode,
              forced,
              cleaned,
              cleanupError,
            },
            null,
            2,
          )}\n`,
          { mode: 0o600 },
        )
      }
      if (cleanupError)
        throw new Error(`${cleanupError}; see ${options.logPath}`)
    })())
  try {
    for (const name of [
      'home',
      'home/.config',
      'home/.local/share',
      'home/.cache',
      'state',
      'tmp',
      'resources',
    ])
      await mkdir(join(root, name), { recursive: true, mode: 0o700 })
    agentPort = await findFreePort()
    const config = join(root, 'launch.json')
    await writeFile(
      config,
      JSON.stringify({
        ports: { server: agentPort, cdp: options.cdpPort },
        directories: {
          resources: 'resources',
          execution: join(options.browser.profile, '.browseros'),
        },
      }),
      { mode: 0o600 },
    )
    await writeFile(paths.tokenFile, `${token}\n`, { mode: 0o600 })
    const log = await open(options.logPath, 'wx', 0o600)
    try {
      child = Bun.spawn({
        cmd: [options.binary, '--config', config],
        cwd: root,
        env: serverEnvironment(paths, options.browser, process.env),
        stdin: 'ignore',
        stdout: log.fd,
        stderr: log.fd,
      })
    } finally {
      await log.close()
    }
    const baseUrl = `http://127.0.0.1:${agentPort}`
    const deadline = Date.now() + 60_000
    while (Date.now() < deadline) {
      if (child.exitCode !== null)
        throw new Error(
          `SDK exited during startup (${child.exitCode}); see ${options.logPath}`,
        )
      if (
        (await readiness(
          { hostname: '127.0.0.1', port: agentPort, path: '/system/ready' },
          false,
        )) &&
        (await readiness(
          {
            socketPath: paths.socketPath,
            path: '/v1/settings',
            headers: { Authorization: `Bearer ${token}` },
          },
          true,
        ))
      ) {
        if (child.exitCode !== null)
          throw new Error('SDK exited while becoming ready')
        readyAt = Date.now()
        return { baseUrl, socketPath: paths.socketPath, token, stop }
      }
      await Bun.sleep(100)
    }
    throw new Error(
      `SDK did not become ready within 60 seconds; see ${options.logPath}`,
    )
  } catch (error) {
    try {
      await stop()
    } catch (cleanup) {
      throw new AggregateError(
        [error, cleanup],
        'SDK startup and cleanup failed',
      )
    }
    throw error
  }
}
