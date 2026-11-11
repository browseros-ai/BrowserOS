/**
 * The Neo SDK image the viewer runs for `bun run neo:viewer`, and every Docker command it
 * issues. One container at a time, under fixed names carrying the viewer's label:
 *
 *   start  remove the previous start's container, network (and data, unless kept)
 *          → pick the image: this checkout's build.sh tag, built when missing or stale
 *          → remove the image the previous build made, if a new one replaced it
 *          → docker run with the README's options, wait until healthy
 *   stop   remove the container and network (and the data, unless kept)
 *
 * The owner runs other Docker work on the same machine, so nothing here touches a container,
 * network or volume without the viewer's label, or an image the viewer did not record building.
 */

import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

export const NAMES = {
  container: 'neo-viewer-sdk',
  network: 'neo-viewer-sdk-net',
  volume: 'neo-viewer-sdk-data',
} as const

/** On everything the viewer creates; cleanup removes a fixed name only when it carries it. */
const LABEL = 'com.browseros.neo-viewer'
/** Ports inside the container: the MCP proxy and `CLAW_HOST_TCP`. */
const MCP_PORT = 9010
const HOST_API_PORT = 9300
const TOKEN_PATH = '/run/neo-viewer/host-token'
const READY_TIMEOUT_MS = 180_000

export interface CommandResult {
  code: number
  stdout: string
  stderr: string
}

/** Runs `docker` (or `git`) with these arguments; `inherit` streams its output to the terminal. */
export type Runner = (
  command: string,
  args: string[],
  options?: { inherit?: boolean },
) => Promise<CommandResult>

export const runCommand: Runner = async (command, args, options = {}) => {
  const child = Bun.spawn([command, ...args], {
    stdout: options.inherit ? 'inherit' : 'pipe',
    stderr: options.inherit ? 'inherit' : 'pipe',
  })
  const [stdout, stderr, code] = await Promise.all([
    options.inherit ? '' : new Response(child.stdout as ReadableStream).text(),
    options.inherit ? '' : new Response(child.stderr as ReadableStream).text(),
    child.exited,
  ])
  return { code, stdout, stderr }
}

/** The image this checkout builds, and a fingerprint of its uncommitted build inputs. */
export interface CheckoutImage {
  tag: string
  /**
   * Null for a clean checkout (the commit names the image). With changes, a hash of the changed
   * build inputs, since a `-dirty` tag stays the same while the changes move.
   */
  fingerprint: string | null
}

/** `build.sh`'s default tag: `neo-sdk:<browser>-<9-char commit>[-dirty]-<arch>`. */
export function imageTag(parts: {
  browserVersion: string
  commit: string
  dirty: boolean
  arch: string
}): string {
  return `neo-sdk:${parts.browserVersion}-${parts.commit.slice(0, 9)}${parts.dirty ? '-dirty' : ''}-${parts.arch}`
}

/** Where `build.sh` lives and the trees whose changes it calls dirty. */
export interface Checkout {
  /** `packages/browseros-agent`. */
  agent: string
  /** `packages/browseros/docker/neo`. */
  neoDocker: string
}

export function checkoutAt(agent: string): Checkout {
  return { agent, neoDocker: join(agent, '..', 'browseros', 'docker', 'neo') }
}

export function buildScript(checkout: Checkout): string {
  return join(
    checkout.agent,
    'crates',
    'neo-sdk',
    'neo-server',
    'docker',
    'build.sh',
  )
}

/**
 * The paths whose changes reach the image: the build context `Dockerfile.dockerignore` keeps
 * (manifests, crates, claw-server) and the image's own folders. A change elsewhere (this tool,
 * say) still makes the tag `-dirty`, as build.sh decides, but needs no rebuild.
 */
function buildInputs(checkout: Checkout): string[] {
  return [
    join(checkout.agent, 'Cargo.toml'),
    join(checkout.agent, 'Cargo.lock'),
    join(checkout.agent, 'crates'),
    join(checkout.agent, 'apps', 'claw-server'),
    checkout.neoDocker,
  ]
}

export async function checkoutImage(
  checkout: Checkout,
  run: Runner,
): Promise<CheckoutImage> {
  const lock = await readFile(
    join(
      checkout.agent,
      'crates',
      'neo-sdk',
      'neo-server',
      'docker',
      'browser.lock',
    ),
    'utf8',
  )
  const browserVersion = /^NEO_BROWSER_VERSION=(\S+)/m.exec(lock)?.[1]
  if (!browserVersion)
    throw new Error('neo-viewer: browser.lock names no NEO_BROWSER_VERSION')
  const git = (args: string[]) => run('git', ['-C', checkout.agent, ...args])
  const commit = (await git(['rev-parse', 'HEAD'])).stdout.trim()
  const dirty =
    (
      await git([
        'status',
        '--porcelain',
        '--',
        checkout.agent,
        checkout.neoDocker,
      ])
    ).stdout.trim() !== ''
  const arch = (
    await run('docker', ['version', '--format', '{{.Server.Arch}}'])
  ).stdout.trim()
  const tag = imageTag({ browserVersion, commit, dirty, arch })
  if (!dirty) return { tag, fingerprint: null }
  const changed = (
    await git([
      'status',
      '--porcelain',
      '-z',
      '--untracked-files=all',
      '--',
      ...buildInputs(checkout),
    ])
  ).stdout
  const root = (await git(['rev-parse', '--show-toplevel'])).stdout.trim()
  const hash = createHash('sha256')
  const paths = changed
    .split('\0')
    .filter((entry) => entry.length > 3)
    .map((entry) => entry.slice(3))
    .sort()
  for (const path of paths) {
    hash.update(`${path}\0`)
    const file = Bun.file(join(root, path))
    hash.update(
      (await file.exists())
        ? new Uint8Array(await file.arrayBuffer())
        : new Uint8Array([0]),
    )
  }
  return { tag, fingerprint: hash.digest('hex') }
}

/** What the command remembers between starts, in the viewer's state folder. */
export interface StartState {
  /** The image the last build made: the only image this command ever removes. */
  lastBuild: { tag: string; id: string; fingerprint: string | null } | null
}

export type ImagePlan =
  | { kind: 'reuse'; tag: string }
  | { kind: 'build'; tag: string; fingerprint: string | null }

/**
 * Reuse the checkout's image when it is on the machine and current: a clean checkout's tag names
 * its commit; a changed checkout's image must be the one this command built from the same
 * changes. Otherwise build.
 */
export function planImage(
  wanted: CheckoutImage,
  existingId: string | null,
  state: StartState,
): ImagePlan {
  if (existingId === null)
    return { kind: 'build', tag: wanted.tag, fingerprint: wanted.fingerprint }
  if (wanted.fingerprint === null) return { kind: 'reuse', tag: wanted.tag }
  const last = state.lastBuild
  const same =
    last?.tag === wanted.tag &&
    last.id === existingId &&
    last.fingerprint === wanted.fingerprint
  return same
    ? { kind: 'reuse', tag: wanted.tag }
    : { kind: 'build', tag: wanted.tag, fingerprint: wanted.fingerprint }
}

/** The image's id, or null when it is not on the machine. */
export async function imageId(
  run: Runner,
  tag: string,
): Promise<string | null> {
  const result = await run('docker', [
    'image',
    'inspect',
    '--format',
    '{{.Id}}',
    tag,
  ])
  return result.code === 0 ? result.stdout.trim() : null
}

/** The newest `neo-sdk:*` image on the machine (for `--no-build`). */
export async function newestImage(run: Runner): Promise<string | null> {
  const result = await run('docker', [
    'images',
    'neo-sdk',
    '--format',
    '{{.Repository}}:{{.Tag}}',
  ])
  const first = result.stdout
    .split('\n')
    .find((line) => line.trim() !== '' && !line.endsWith(':<none>'))
  return first?.trim() ?? null
}

/** Removes the image the previous build made, once a new build replaced it. Never any other. */
export async function removeReplacedImage(
  run: Runner,
  previous: StartState['lastBuild'],
  newId: string,
): Promise<string | null> {
  if (!previous || previous.id === newId) return null
  const result = await run('docker', ['image', 'rm', previous.id])
  return result.code === 0
    ? `${previous.tag} (${previous.id.slice(7, 19)})`
    : null
}

type Kind = 'container' | 'network' | 'volume'

/**
 * Removes what an earlier start left under the viewer's names: the container, the network and,
 * unless `keepData`, the volume (the browser profile). A name in use by something without the
 * viewer's label is not the viewer's: the start stops rather than touch it.
 */
export async function removeLeftovers(
  run: Runner,
  keepData: boolean,
): Promise<string[]> {
  const removed: string[] = []
  const targets: Array<[Kind, string]> = [
    ['container', NAMES.container],
    ['network', NAMES.network],
    ...(keepData ? [] : ([['volume', NAMES.volume]] as Array<[Kind, string]>)),
  ]
  for (const [kind, name] of targets) {
    const inspect = await run('docker', [
      kind,
      'inspect',
      '--format',
      `{{index .${kind === 'container' ? 'Config.' : ''}Labels "${LABEL}"}}`,
      name,
    ])
    if (inspect.code !== 0) continue
    if (inspect.stdout.trim() !== '1') {
      throw new Error(
        `neo-viewer: a ${kind} named ${name} exists but the viewer did not create it; remove or rename it`,
      )
    }
    const args = kind === 'container' ? ['rm', '-f', name] : [kind, 'rm', name]
    const result = await run('docker', args)
    if (result.code !== 0)
      throw new Error(
        `neo-viewer: could not remove ${kind} ${name}: ${result.stderr.trim()}`,
      )
    removed.push(`${kind} ${name}`)
  }
  return removed
}

export interface RunningContainer {
  image: string
  /** Neo's MCP endpoint as published on this machine. */
  mcpUrl: URL
  /** The host API (`CLAW_HOST_TCP`) as published on this machine. */
  hostApiUrl: URL
}

/**
 * `docker run` as the image's README shows for a standalone start: its own network, the MCP
 * port with `NEO_MCP_REMOTE=true`, and the host API over `CLAW_HOST_TCP` with the token file,
 * both published on 127.0.0.1 at ports Docker picks. Inside the container `CLAW_HOST_TCP` binds
 * 0.0.0.0: bound to 127.0.0.1 the published port reaches nothing (measured).
 */
export async function startContainer(
  run: Runner,
  options: { image: string; tokenFile: string },
): Promise<RunningContainer> {
  const label = `${LABEL}=1`
  const network = await run('docker', [
    'network',
    'create',
    '--label',
    label,
    NAMES.network,
  ])
  if (network.code !== 0)
    throw new Error(
      `neo-viewer: docker network create failed: ${network.stderr.trim()}`,
    )
  const volume = await run('docker', [
    'volume',
    'create',
    '--label',
    label,
    NAMES.volume,
  ])
  if (volume.code !== 0)
    throw new Error(
      `neo-viewer: docker volume create failed: ${volume.stderr.trim()}`,
    )
  const started = await run('docker', [
    'run',
    '-d',
    '--name',
    NAMES.container,
    '--label',
    label,
    '--network',
    NAMES.network,
    '--shm-size=1g',
    '--stop-timeout',
    '25',
    '-e',
    'NEO_MCP_REMOTE=true',
    '-e',
    `CLAW_HOST_TCP=0.0.0.0:${HOST_API_PORT}`,
    '-e',
    `CLAW_HOST_TOKEN_FILE=${TOKEN_PATH}`,
    '-v',
    `${options.tokenFile}:${TOKEN_PATH}:ro`,
    '-v',
    `${NAMES.volume}:/data`,
    '-p',
    `127.0.0.1::${MCP_PORT}`,
    '-p',
    `127.0.0.1::${HOST_API_PORT}`,
    options.image,
  ])
  if (started.code !== 0)
    throw new Error(`neo-viewer: docker run failed: ${started.stderr.trim()}`)
  await waitUntilHealthy(run)
  return {
    image: options.image,
    mcpUrl: new URL(`http://${await publishedAddress(run, MCP_PORT)}/mcp`),
    hostApiUrl: new URL(`http://${await publishedAddress(run, HOST_API_PORT)}`),
  }
}

/** Waits for the image's HEALTHCHECK (entrypoint done, `/system/ready` answers). */
async function waitUntilHealthy(run: Runner): Promise<void> {
  const deadline = Date.now() + READY_TIMEOUT_MS
  for (;;) {
    const state = await run('docker', [
      'inspect',
      '--format',
      '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}',
      NAMES.container,
    ])
    const [status, health] = state.stdout.trim().split(' ')
    if (health === 'healthy') return
    if (
      status !== 'running' ||
      health === 'unhealthy' ||
      Date.now() > deadline
    ) {
      const logs = await run('docker', [
        'logs',
        '--tail',
        '40',
        NAMES.container,
      ])
      throw new Error(
        `neo-viewer: the container did not come up (${status ?? 'gone'}, ${health ?? 'no health'}). Its last log lines:\n${logs.stdout}${logs.stderr}`,
      )
    }
    await Bun.sleep(1000)
  }
}

async function publishedAddress(run: Runner, port: number): Promise<string> {
  const result = await run('docker', ['port', NAMES.container, `${port}/tcp`])
  const address = result.stdout
    .split('\n')
    .find((line) => line.startsWith('127.0.0.1:'))
  if (!address)
    throw new Error(`neo-viewer: port ${port} is not published on 127.0.0.1`)
  return address.trim()
}

/**
 * Stops and removes the container and its network; the volume too, unless `keepData`. A kept
 * profile is stopped first with the image's 25 s grace, so the browser saves its cookies.
 */
export async function stopContainer(
  run: Runner,
  keepData: boolean,
): Promise<void> {
  if (keepData) await run('docker', ['stop', '--time', '25', NAMES.container])
  await removeLeftovers(run, keepData)
}

/** Whether the viewer's container is running, for the page. */
export async function containerUp(run: Runner): Promise<boolean> {
  const result = await run('docker', [
    'inspect',
    '--format',
    '{{.State.Running}}',
    NAMES.container,
  ])
  return result.code === 0 && result.stdout.trim() === 'true'
}
