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

/**
 * The names one instance's objects go under. Every start, cleanup and Ctrl-C is scoped to one
 * instance (`--name`, default `default`), so viewers started side by side under different names
 * never touch each other's container, network, data or state.
 */
export interface InstanceNames {
  instance: string
  container: string
  network: string
  volume: string
}

/** What `--name` accepts: Docker's name characters, short enough for the derived names. */
export const INSTANCE_PATTERN = /^[a-z0-9][a-z0-9_.-]{0,39}$/

export function instanceNames(instance: string): InstanceNames {
  return {
    instance,
    container: `neo-viewer-${instance}`,
    network: `neo-viewer-${instance}-net`,
    volume: `neo-viewer-${instance}-data`,
  }
}

/** On everything the viewer creates; cleanup removes a fixed name only when it carries it. */
const LABEL = 'com.browseros.neo-viewer'
/** Ports inside the container: the MCP proxy and `CLAW_HOST_TCP`. */
const MCP_PORT = 9010
const HOST_API_PORT = 9300
const TOKEN_PATH = '/run/neo-viewer/host-token'
/**
 * `CLAW_TRACE_VALUE_BYTES`: how much of an operation's arguments and result Neo's trace keeps
 * whole. Its default is 2 KiB (results) and 4 KiB (arguments), so a snapshot's text, the very
 * thing the operation panel shows, was cut. The viewer asks for Neo's own reply limit, 256 KiB,
 * so an operation is kept as whole as a reply. An image older than the setting ignores it.
 */
export const TRACE_VALUE_BYTES = 256 * 1024
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
  /** The tag of this commit without its changes, whose image is the same when no input changed. */
  cleanTag: string
  /**
   * Null for a clean checkout (the commit names the image). With changes, a hash of the changed
   * build inputs, since a `-dirty` tag stays the same while the changes move.
   */
  fingerprint: string | null
  /** Whether any changed file is a build input (`buildInputs`); false when clean. */
  buildInputsChanged: boolean
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

/**
 * Where `build.sh` lives and the tree whose changes it calls dirty: `packages/browseros-agent`,
 * which holds every input of the Neo SDK image (its supervisor too, since it stopped sharing the
 * default image's folder).
 */
export interface Checkout {
  /** `packages/browseros-agent`. */
  agent: string
}

export function checkoutAt(agent: string): Checkout {
  return { agent }
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
 * (manifests, crates with the image's own folder, claw-server). A change elsewhere (this tool,
 * say) still makes the tag `-dirty`, as build.sh decides, but needs no rebuild.
 */
function buildInputs(checkout: Checkout): string[] {
  return [
    join(checkout.agent, 'Cargo.toml'),
    join(checkout.agent, 'Cargo.lock'),
    join(checkout.agent, 'crates'),
    join(checkout.agent, 'apps', 'claw-server'),
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
      await git(['status', '--porcelain', '--', checkout.agent])
    ).stdout.trim() !== ''
  const arch = (
    await run('docker', ['version', '--format', '{{.Server.Arch}}'])
  ).stdout.trim()
  const tag = imageTag({ browserVersion, commit, dirty, arch })
  const cleanTag = imageTag({ browserVersion, commit, dirty: false, arch })
  if (!dirty)
    return { tag, cleanTag, fingerprint: null, buildInputsChanged: false }
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
  return {
    tag,
    cleanTag,
    fingerprint: hash.digest('hex'),
    buildInputsChanged: paths.length > 0,
  }
}

/** What the command remembers between starts, in the viewer's state folder. */
export interface StartState {
  /** The image the last build made: with `kept`, the only images this instance ever removes. */
  lastBuild: { tag: string; id: string; fingerprint: string | null } | null
  /**
   * Images this instance's builds replaced that could not be removed yet (a container, or another
   * instance, still used them); tried again at each start until nothing does.
   */
  kept?: ImageRef[]
}

export interface ImageRef {
  tag: string
  id: string
}

/** An image left in place, and why (a container uses it, another instance names it, or Docker's refusal). */
export type KeptImage = ImageRef & { reason: string }

export type ImagePlan =
  | { kind: 'reuse'; tag: string }
  | { kind: 'build'; tag: string; fingerprint: string | null }

/**
 * Reuse the checkout's image when it is on the machine and current: a clean checkout's tag names
 * its commit; a checkout whose changes touch no build input runs the same server as its commit,
 * so the commit's image serves when present; otherwise a changed checkout's image must be the one
 * this command built from the same changes. Else build.
 */
export function planImage(
  wanted: CheckoutImage,
  existing: { tag: string | null; cleanTag: string | null },
  state: StartState,
): ImagePlan {
  const build: ImagePlan = {
    kind: 'build',
    tag: wanted.tag,
    fingerprint: wanted.fingerprint,
  }
  if (wanted.fingerprint === null)
    return existing.tag ? { kind: 'reuse', tag: wanted.tag } : build
  if (!wanted.buildInputsChanged && existing.cleanTag)
    return { kind: 'reuse', tag: wanted.cleanTag }
  const last = state.lastBuild
  const same =
    existing.tag !== null &&
    last?.tag === wanted.tag &&
    last.id === existing.tag &&
    last.fingerprint === wanted.fingerprint
  return same ? { kind: 'reuse', tag: wanted.tag } : build
}

/** The image's id, or null when it is not on the machine. */
export async function imageId(
  run: Runner,
  tag: string,
): Promise<string | null> {
  const found = await lookUpImage(run, tag)
  return 'id' in found ? found.id : null
}

/**
 * The image's id; `missing` only when Docker says there is no such image, so a Docker that did
 * not answer is never taken for an image that is gone.
 */
async function lookUpImage(
  run: Runner,
  ref: string,
): Promise<{ id: string } | { missing: true } | { error: string }> {
  const result = await run('docker', [
    'image',
    'inspect',
    '--format',
    '{{.Id}}',
    ref,
  ])
  if (result.code === 0) return { id: result.stdout.trim() }
  return /no such (image|object)/i.test(result.stderr)
    ? { missing: true }
    : { error: firstLine(result.stderr) }
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

/**
 * Why an image must stay, or null when nothing holds it. Instances on one commit share one image
 * tag, so the image one instance's build replaced can be what another instance's container runs:
 * any container on the machine, running or stopped, holds its image, and so does another
 * instance's last build, which its next start would reuse. Another instance's `kept` list does
 * not: those are images it means to remove, and counting them would let two instances that kept
 * the same image wait on each other forever.
 */
async function imageHolder(
  run: Runner,
  image: ImageRef,
  otherStates: StartState[],
): Promise<string | null> {
  const containers = await run('docker', [
    'ps',
    '-a',
    '--filter',
    `ancestor=${image.id}`,
    '--format',
    '{{.Names}}',
  ])
  // Unknown is not free: keep it, and try again next start.
  if (containers.code !== 0)
    return `docker ps failed (${firstLine(containers.stderr)})`
  const names = containers.stdout
    .split('\n')
    .filter((name) => name.trim() !== '')
  if (names.length > 0) return `used by container ${names.join(', ')}`
  const named = otherStates.some((state) => state.lastBuild?.id === image.id)
  return named ? "another instance's last build" : null
}

/**
 * Removes the images this instance's builds replaced, each only when nothing holds it
 * (`imageHolder`), and never by force: Docker refuses a plain `image rm` of an image a container
 * uses, but a forced one deletes it under that container. What stays is kept for the next start.
 */
export async function retireImages(
  run: Runner,
  images: ImageRef[],
  otherStates: StartState[],
): Promise<{ removed: ImageRef[]; kept: KeptImage[] }> {
  const removed: ImageRef[] = []
  const kept: KeptImage[] = []
  for (const image of images) {
    const found = await lookUpImage(run, image.id)
    if ('missing' in found) continue
    if ('error' in found) {
      kept.push({
        ...image,
        reason: `docker image inspect failed (${found.error})`,
      })
      continue
    }
    const holder = await imageHolder(run, image, otherStates)
    if (holder) {
      kept.push({ ...image, reason: holder })
      continue
    }
    const result = await run('docker', ['image', 'rm', image.id])
    if (result.code === 0) removed.push(image)
    else kept.push({ ...image, reason: firstLine(result.stderr) })
  }
  return { removed, kept }
}

/**
 * A start's image bookkeeping, once it knows the image it runs: retires what this instance's
 * builds replaced (its previous build when this start built a new one, and what earlier starts
 * had to keep) and returns the state to save. The image this start runs is never retired; when it
 * is one an earlier start kept, it stays on the kept list, so a later start still removes it.
 */
export async function settleImages(
  run: Runner,
  state: StartState,
  start: { built: StartState['lastBuild']; running: string },
  otherStates: StartState[],
): Promise<{ state: StartState; removed: ImageRef[]; kept: KeptImage[] }> {
  const replaced = new Map<string, ImageRef>()
  if (start.built && state.lastBuild)
    replaced.set(state.lastBuild.id, {
      tag: state.lastBuild.tag,
      id: state.lastBuild.id,
    })
  for (const image of state.kept ?? [])
    replaced.set(image.id, { tag: image.tag, id: image.id })
  const running = replaced.get(start.running)
  replaced.delete(start.running)
  const carried =
    running && state.kept?.some((image) => image.id === running.id)
      ? [running]
      : []
  const { removed, kept } = await retireImages(
    run,
    [...replaced.values()],
    otherStates,
  )
  return {
    state: {
      lastBuild: start.built ?? state.lastBuild,
      kept: [...carried, ...kept.map(({ tag, id }) => ({ tag, id }))],
    },
    removed,
    kept,
  }
}

function firstLine(text: string): string {
  return text.trim().split('\n')[0] || 'docker refused'
}

/** `neo-sdk:… (0123456789ab)`. */
export function imageLabel(image: ImageRef): string {
  return `${image.tag} (${image.id.slice(7, 19)})`
}

type Kind = 'container' | 'network' | 'volume'

/** Marks one launch's objects, so its Ctrl-C removes what it made and not a later launch's. */
const LAUNCH_LABEL = `${LABEL}.launch`
/** Marks the instance an object belongs to; a start removes only its own instance's leftovers. */
const INSTANCE_LABEL = `${LABEL}.instance`

/**
 * A named object's own reference (a container's or network's id; a volume's name) and one of its
 * labels; `undefined` when no object has that name. Later commands use the reference, so an object
 * another launch put under the same name meanwhile is never the one acted on.
 */
async function inspectLabel(
  run: Runner,
  kind: Kind,
  name: string,
  key: string,
): Promise<{ ref: string; label: string } | undefined> {
  const labels = kind === 'container' ? '.Config.Labels' : '.Labels'
  const ref = kind === 'volume' ? '{{.Name}}' : '{{.Id}}'
  const inspect = await run('docker', [
    kind,
    'inspect',
    '--format',
    `${ref}|{{index ${labels} "${key}"}}`,
    name,
  ])
  if (inspect.code !== 0) return undefined
  const [found = '', label = ''] = inspect.stdout.trim().split('|')
  return { ref: found, label }
}

async function remove(
  run: Runner,
  kind: Kind,
  ref: string,
  keepData: boolean,
): Promise<void> {
  // A kept profile needs the image's graceful stop (25 s), so the browser saves its cookies.
  if (kind === 'container' && keepData)
    await run('docker', ['stop', '--time', '25', ref])
  const result = await run(
    'docker',
    kind === 'container' ? ['rm', '-f', ref] : [kind, 'rm', ref],
  )
  if (result.code !== 0) {
    throw new Error(
      `neo-viewer: could not remove ${kind} ${ref}: ${result.stderr.trim()}`,
    )
  }
}

/**
 * Removes what an earlier start left under the viewer's names: the container, the network and,
 * unless `keepData`, the volume (the browser profile). Every name is checked, the kept volume
 * too: one in use by something without the viewer's label is not the viewer's, and the start
 * stops rather than touch or mount it.
 */
export async function removeLeftovers(
  run: Runner,
  names: InstanceNames,
  keepData: boolean,
): Promise<string[]> {
  const removed: string[] = []
  const targets: Array<[Kind, string]> = [
    ['container', names.container],
    ['network', names.network],
    ['volume', names.volume],
  ]
  for (const [kind, name] of targets) {
    const found = await inspectLabel(run, kind, name, LABEL)
    if (found === undefined) continue
    const instance = await inspectLabel(run, kind, found.ref, INSTANCE_LABEL)
    if (found.label !== '1' || instance?.label !== names.instance) {
      throw new Error(
        `neo-viewer: a ${kind} named ${name} exists but this viewer instance did not create it; remove or rename it`,
      )
    }
    if (kind === 'volume' && keepData) continue
    await remove(run, kind, found.ref, keepData)
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
  options: {
    names: InstanceNames
    image: string
    tokenFile: string
    launch: string
  },
): Promise<RunningContainer> {
  const labels = [
    '--label',
    `${INSTANCE_LABEL}=${options.names.instance}`,
    '--label',
    `${LABEL}=1`,
    '--label',
    `${LAUNCH_LABEL}=${options.launch}`,
  ]
  await createNetwork(run, options.names, labels)
  // With --keep-data the volume (checked as the viewer's) is reused; create is then a no-op.
  const volume = await run('docker', [
    'volume',
    'create',
    ...labels,
    options.names.volume,
  ])
  if (volume.code !== 0)
    throw new Error(
      `neo-viewer: docker volume create failed: ${volume.stderr.trim()}`,
    )
  const started = await run('docker', [
    'run',
    '-d',
    '--name',
    options.names.container,
    ...labels,
    '--network',
    options.names.network,
    '--shm-size=1g',
    '--stop-timeout',
    '25',
    // On Linux a bind-mounted file keeps its host owner, so the 0600 token is readable only by
    // that uid: Neo runs as it (the image's PUID/PGID). Docker Desktop on macOS needs no mapping.
    ...ownerIds(),
    '-e',
    'NEO_MCP_REMOTE=true',
    '-e',
    `CLAW_HOST_TCP=0.0.0.0:${HOST_API_PORT}`,
    '-e',
    `CLAW_HOST_TOKEN_FILE=${TOKEN_PATH}`,
    '-e',
    `CLAW_TRACE_VALUE_BYTES=${TRACE_VALUE_BYTES}`,
    '-v',
    `${options.tokenFile}:${TOKEN_PATH}:ro`,
    '-v',
    `${options.names.volume}:/data`,
    '-p',
    `127.0.0.1::${MCP_PORT}`,
    '-p',
    `127.0.0.1::${HOST_API_PORT}`,
    options.image,
  ])
  if (started.code !== 0)
    throw new Error(`neo-viewer: docker run failed: ${started.stderr.trim()}`)
  await waitUntilHealthy(run, options.names)
  return {
    image: options.image,
    mcpUrl: new URL(
      `http://${await publishedAddress(run, options.names, MCP_PORT)}/mcp`,
    ),
    hostApiUrl: new URL(
      `http://${await publishedAddress(run, options.names, HOST_API_PORT)}`,
    ),
  }
}

/** Explicit subnets tried when Docker's own pools are used up: 10.213.0.0/24 … 10.213.255.0/24. */
const FALLBACK_SUBNETS = Array.from(
  { length: 256 },
  (_, n) => `10.213.${n}.0/24`,
)

/**
 * The container's own network, as the image's README asks for with `NEO_MCP_REMOTE=true`. A
 * machine with many Docker networks can use up Docker's default address pools ("all predefined
 * address pools have been fully subnetted"); then a /24 is named explicitly, and Docker refuses
 * any that overlaps a network it already has.
 */
export async function createNetwork(
  run: Runner,
  names: InstanceNames,
  labels: string[],
): Promise<void> {
  const create = (extra: string[]) =>
    run('docker', ['network', 'create', ...labels, ...extra, names.network])
  const first = await create([])
  if (first.code === 0) return
  if (!/fully subnetted|address pools/i.test(first.stderr)) {
    throw new Error(
      `neo-viewer: docker network create failed: ${first.stderr.trim()}`,
    )
  }
  for (const subnet of FALLBACK_SUBNETS) {
    if ((await create(['--subnet', subnet])).code === 0) {
      console.log(
        `Docker's address pools are used up; the network uses ${subnet}`,
      )
      return
    }
  }
  throw new Error(
    `neo-viewer: Docker has no free network addresses (${first.stderr.trim()}); remove networks you no longer use`,
  )
}

function ownerIds(): string[] {
  if (process.platform !== 'linux' || !process.getuid || !process.getgid)
    return []
  return ['-e', `PUID=${process.getuid()}`, '-e', `PGID=${process.getgid()}`]
}

/** Waits for the image's HEALTHCHECK (entrypoint done, `/system/ready` answers). */
async function waitUntilHealthy(
  run: Runner,
  names: InstanceNames,
): Promise<void> {
  const deadline = Date.now() + READY_TIMEOUT_MS
  for (;;) {
    const state = await run('docker', [
      'inspect',
      '--format',
      '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}',
      names.container,
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
        names.container,
      ])
      throw new Error(
        `neo-viewer: the container did not come up (${status ?? 'gone'}, ${health ?? 'no health'}). Its last log lines:\n${logs.stdout}${logs.stderr}`,
      )
    }
    await Bun.sleep(1000)
  }
}

async function publishedAddress(
  run: Runner,
  names: InstanceNames,
  port: number,
): Promise<string> {
  const result = await run('docker', ['port', names.container, `${port}/tcp`])
  const address = result.stdout
    .split('\n')
    .find((line) => line.startsWith('127.0.0.1:'))
  if (!address)
    throw new Error(`neo-viewer: port ${port} is not published on 127.0.0.1`)
  return address.trim()
}

/**
 * Ctrl-C: removes the container and network this launch created (and its volume, unless
 * `keepData`). Each must still carry this launch's label: another `bun run neo:viewer` started
 * since has replaced them, and they are its to remove.
 */
export async function stopContainer(
  run: Runner,
  names: InstanceNames,
  launch: string,
  keepData: boolean,
): Promise<void> {
  const targets: Array<[Kind, string]> = [
    ['container', names.container],
    ['network', names.network],
    ...(keepData ? [] : ([['volume', names.volume]] as Array<[Kind, string]>)),
  ]
  for (const [kind, name] of targets) {
    const found = await inspectLabel(run, kind, name, LAUNCH_LABEL)
    if (found?.label !== launch) continue
    await remove(run, kind, found.ref, keepData)
  }
}

/** Whether the viewer's container is running, for the page. */
export async function containerUp(
  run: Runner,
  names: InstanceNames,
): Promise<boolean> {
  const result = await run('docker', [
    'inspect',
    '--format',
    '{{.State.Running}}',
    names.container,
  ])
  return result.code === 0 && result.stdout.trim() === 'true'
}
