/**
 * `bun run neo:viewer`, once `main.ts` has found the tool's dependencies: the whole loop in one
 * command. It builds this checkout's Neo SDK image when needed, starts it fresh, serves the viewer
 * against it and prints where to point the agent and the browser. Ctrl-C stops the viewer and
 * removes what it started. With `--upstream` it starts no container and only serves the viewer.
 * Everything it creates or removes belongs to one instance (`--name`).
 */

import { randomBytes } from 'node:crypto'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { type LaunchOptions, launchOptionsFrom, stateRoot } from './config'
import { describeError } from './errors'
import type { HostApiConfig } from './hostApi'
import { LOG_FILE } from './log/run-log'
import {
  buildScript,
  checkoutAt,
  checkoutImage,
  imageId,
  imageLabel,
  instanceNames,
  newestImage,
  planImage,
  type Runner,
  removeLeftovers,
  runCommand,
  type StartState,
  settleImages,
  startContainer,
  stopContainer,
} from './neo-container'
import { buildPage } from './page'
import {
  claimStateFolder,
  liveViewer,
  otherInstanceStates,
  portFree,
  readViewerFile,
} from './running-viewer'
import { type RunningViewer, startViewer } from './viewer-server'

const AGENT_DIR = join(import.meta.dir, '..', '..', '..')

const options = launchOptionsFrom(process.argv.slice(2), process.env)
const names = instanceNames(options.instance)
/** The instance's live viewer: written once this one serves, so a later start can tell. */
const viewerFile = join(options.dataDir, 'viewer.json')
let viewer: RunningViewer | null = null
let ownsContainer = false
let currentImage = ''
/** Labels this launch's container, network and volume, so its Ctrl-C removes only those. */
const launchId = randomBytes(6).toString('hex')
/** When cleanup began; null until then. */
let stoppingSince: number | null = null
/**
 * Ctrl-C reaches this process more than once: the terminal signals the whole process group, and
 * each `bun run` above it (`neo:viewer` → `start`) forwards the signal again. Repeats within this
 * window are those copies and must not cut the cleanup short; a later Ctrl-C quits at once.
 */
const REPEATED_SIGNAL_MS = 3000

/** Set by Ctrl-C: startup issues no further Docker command. */
let cancelled = false
/** The Docker command startup has in flight; cleanup waits for it, so nothing it creates is missed. */
let inFlight: Promise<unknown> = Promise.resolve()

/** Startup's Docker and git commands: refused once Ctrl-C began, and each one tracked. */
const startupRun: Runner = (command, args, runOptions) => {
  if (cancelled) return Promise.reject(new Error('neo-viewer: start cancelled'))
  const result = runCommand(command, args, runOptions)
  inFlight = result.catch(() => {})
  return result
}

for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => void stop(0))
}

try {
  await mkdir(options.dataDir, { recursive: true })
  await claimStateFolder(options.dataDir, options.instance)
  await checkRunningViewer()
  if (!portFree(options.port)) {
    throw new Error(
      `neo-viewer: port ${options.port} is in use on 127.0.0.1 (another viewer?); stop it, or pick another --port`,
    )
  }
  // Before any Docker work: a missing or broken package fails here, in a second.
  const page = await buildPage()
  const neo = options.upstream
    ? { mcpUrl: options.upstream, hostApi: options.hostApi }
    : await startNeo(options)
  viewer = await startViewer({
    page,
    hostApi: neo.hostApi,
    port: options.port,
    upstream: neo.mcpUrl,
    dataDir: options.dataDir,
    container: ownsContainer ? { names, image: currentImage } : null,
    instance: options.instance,
  })
  console.log(`
  Viewer:         ${viewer.url}
  Agent MCP URL:  ${viewer.mcpUrl}
  Claude Code:    claude mcp add --transport http neo-viewer ${viewer.mcpUrl}
`)
  await writeFile(
    viewerFile,
    // The port it listens on (not --port, which 0 leaves to the OS), for the same-port check.
    JSON.stringify({
      pid: process.pid,
      port: Number(new URL(viewer.url).port),
      url: viewer.url,
    }),
  )
} catch (error) {
  // A start cut short by Ctrl-C is already being cleaned up by `stop`.
  if (!cancelled) {
    console.error(describeError(error))
    await stop(1)
  }
}

/** Picks or builds the image, clears the last start, starts the container; returns where Neo answers. */
async function startNeo(
  launch: LaunchOptions,
): Promise<{ mcpUrl: URL; hostApi: HostApiConfig }> {
  const run = startupRun
  const removed = await removeLeftovers(run, names, launch.keepData)
  if (removed.length > 0)
    console.log(`Removed the last start's ${removed.join(', ')}`)
  currentImage = await chooseImage(launch)
  if (!launch.keepData)
    await Promise.all(
      // The run log starts empty, as the browser profile does; `tasks/` is an older viewer's log.
      [LOG_FILE, 'tasks'].map((name) =>
        rm(join(launch.dataDir, name), { recursive: true, force: true }),
      ),
    )
  const tokenFile = join(launch.dataDir, 'host-token')
  await writeFile(tokenFile, randomBytes(32).toString('hex'), { mode: 0o600 })
  console.log(`Starting ${currentImage} as ${names.container}…`)
  ownsContainer = true
  const container = await startContainer(run, {
    names,
    image: currentImage,
    tokenFile,
    launch: launchId,
  })
  return {
    mcpUrl: container.mcpUrl,
    hostApi: { url: container.hostApiUrl, tokenFile },
  }
}

/** `--image`, `--no-build`'s newest image, or this checkout's image (built when missing or stale). */
async function chooseImage(launch: LaunchOptions): Promise<string> {
  const run = startupRun
  if (launch.image) {
    if ((await imageId(run, launch.image)) === null)
      throw new Error(`neo-viewer: no image ${launch.image} on this machine`)
    return launch.image
  }
  if (!launch.build) {
    const newest = await newestImage(run)
    if (!newest)
      throw new Error(
        'neo-viewer: no neo-sdk image on this machine; run without --no-build',
      )
    return newest
  }
  const checkout = checkoutAt(AGENT_DIR)
  const wanted = await checkoutImage(checkout, run)
  const statePath = join(launch.dataDir, 'start-state.json')
  const state = await readState(statePath)
  const plan = planImage(
    wanted,
    {
      tag: await imageId(run, wanted.tag),
      cleanTag: await imageId(run, wanted.cleanTag),
    },
    state,
  )
  /** Set when this start built the image. */
  let newBuild: StartState['lastBuild'] = null
  let currentId: string | null
  if (plan.kind === 'reuse') {
    console.log(`Using ${plan.tag}, already built`)
    currentId = await imageId(run, plan.tag)
  } else {
    console.log(
      `Building ${plan.tag} with build.sh (a first build takes about 7 minutes)…`,
    )
    const built = await run(
      'bash',
      [buildScript(checkout), '--tag', plan.tag],
      { inherit: true },
    )
    if (built.code !== 0)
      throw new Error(`neo-viewer: build.sh failed (exit ${built.code})`)
    currentId = await imageId(run, plan.tag)
    if (!currentId)
      throw new Error(
        `neo-viewer: build.sh finished but ${plan.tag} is not on the machine`,
      )
    newBuild = { tag: plan.tag, id: currentId, fingerprint: plan.fingerprint }
  }
  // Without the id of the image this start runs, cleanup could not tell it from the ones to retire.
  if (!currentId) {
    console.log(
      `Could not read ${plan.tag}'s id; no image is removed this start`,
    )
    return plan.tag
  }
  const settled = await settleImages(
    run,
    state,
    { built: newBuild, running: currentId },
    await otherInstanceStates(launch.dataDir, stateRoot(process.env)),
  )
  for (const image of settled.removed)
    console.log(`Removed the image an earlier build made: ${imageLabel(image)}`)
  for (const image of settled.kept)
    console.log(`Kept ${imageLabel(image)}: ${image.reason}`)
  await writeFile(statePath, JSON.stringify(settled.state, null, 2))
  return plan.tag
}

async function readState(path: string): Promise<StartState> {
  try {
    return JSON.parse(await readFile(path, 'utf8')) as StartState
  } catch {
    return { lastBuild: null }
  }
}

/** Ctrl-C, or a failed start: stop the viewer, then remove the container and network it started. */
async function stop(code: number): Promise<never> {
  if (stoppingSince !== null) {
    if (Date.now() - stoppingSince < REPEATED_SIGNAL_MS)
      return new Promise<never>(() => {})
    console.error(
      `Quitting without finishing cleanup; remove ${names.container} and ${names.network} with docker rm -f / docker network rm`,
    )
    process.exit(code)
  }
  stoppingSince = Date.now()
  cancelled = true
  await inFlight
  await viewer?.stop()
  await forgetViewer()
  if (ownsContainer) {
    console.log(
      `Removing ${names.container}${options.keepData ? '' : ' and its data'}…`,
    )
    await stopContainer(runCommand, names, launchId, options.keepData).catch(
      (error) => console.error(String(error)),
    )
  }
  process.exit(code)
}

/**
 * Another viewer of this instance, still running? On the same port the start would break it and
 * then fail to listen, so it stops here, touching nothing. On another port it goes on and replaces
 * that viewer's container, as any start of the same instance does, and says so.
 */
async function checkRunningViewer(): Promise<void> {
  const running = await liveViewer(viewerFile)
  if (!running) return
  if (running.port === options.port) {
    throw new Error(
      `neo-viewer: instance ${options.instance} is already running (pid ${running.pid}, ${running.url}); stop it with Ctrl-C there, or start another instance with --name`,
    )
  }
  console.log(
    `Instance ${options.instance} is running in pid ${running.pid} (${running.url}); this start replaces its container.`,
  )
}

/** Removes the live-viewer file, if it is still this process's. */
async function forgetViewer(): Promise<void> {
  const recorded = await readViewerFile(viewerFile)
  if (recorded?.pid === process.pid) await rm(viewerFile, { force: true })
}
