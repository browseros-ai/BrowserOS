/**
 * `bun run neo:viewer`: the whole loop in one command. It builds this checkout's Neo SDK image
 * when needed, starts it fresh, serves the viewer against it and prints where to point the agent
 * and the browser. Ctrl-C stops the viewer and removes what it started. With `--upstream` it
 * starts no container and only serves the viewer.
 */

import { randomBytes } from 'node:crypto'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { type LaunchOptions, launchOptionsFrom } from './config'
import type { HostApiConfig } from './hostApi'
import {
  buildScript,
  checkoutAt,
  checkoutImage,
  imageId,
  NAMES,
  newestImage,
  planImage,
  removeLeftovers,
  removeReplacedImage,
  runCommand,
  type StartState,
  startContainer,
  stopContainer,
} from './neo-container'
import { type RunningViewer, startViewer } from './viewer-server'

const AGENT_DIR = join(import.meta.dir, '..', '..', '..')

const options = launchOptionsFrom(process.argv.slice(2), process.env)
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

for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => void stop(0))
}

try {
  const neo = options.upstream
    ? { mcpUrl: options.upstream, hostApi: options.hostApi }
    : await startNeo(options)
  viewer = await startViewer({
    hostApi: neo.hostApi,
    port: options.port,
    upstream: neo.mcpUrl,
    dataDir: options.dataDir,
    container: ownsContainer
      ? { name: NAMES.container, image: currentImage }
      : null,
  })
  console.log(`
  Viewer:         ${viewer.url}
  Agent MCP URL:  ${viewer.mcpUrl}
  Claude Code:    claude mcp add --transport http neo-viewer ${viewer.mcpUrl}
`)
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  await stop(1)
}

/** Picks or builds the image, clears the last start, starts the container; returns where Neo answers. */
async function startNeo(
  launch: LaunchOptions,
): Promise<{ mcpUrl: URL; hostApi: HostApiConfig }> {
  await mkdir(launch.dataDir, { recursive: true })
  const run = runCommand
  const removed = await removeLeftovers(run, launch.keepData)
  if (removed.length > 0)
    console.log(`Removed the last start's ${removed.join(', ')}`)
  currentImage = await chooseImage(launch)
  if (!launch.keepData)
    await rm(join(launch.dataDir, 'tasks'), { recursive: true, force: true })
  const tokenFile = join(launch.dataDir, 'host-token')
  await writeFile(tokenFile, randomBytes(32).toString('hex'), { mode: 0o600 })
  console.log(`Starting ${currentImage} as ${NAMES.container}…`)
  ownsContainer = true
  const container = await startContainer(run, {
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
  const run = runCommand
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
  if (plan.kind === 'reuse') {
    console.log(`Using ${plan.tag}, already built`)
    return plan.tag
  }
  console.log(
    `Building ${plan.tag} with build.sh (a first build takes about 7 minutes)…`,
  )
  const built = await run('bash', [buildScript(checkout), '--tag', plan.tag], {
    inherit: true,
  })
  if (built.code !== 0)
    throw new Error(`neo-viewer: build.sh failed (exit ${built.code})`)
  const id = await imageId(run, plan.tag)
  if (!id)
    throw new Error(
      `neo-viewer: build.sh finished but ${plan.tag} is not on the machine`,
    )
  const replaced = await removeReplacedImage(run, state.lastBuild, id)
  if (replaced)
    console.log(`Removed the image the last build made: ${replaced}`)
  await writeFile(
    statePath,
    JSON.stringify(
      {
        lastBuild: { tag: plan.tag, id, fingerprint: plan.fingerprint },
      } satisfies StartState,
      null,
      2,
    ),
  )
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
      `Quitting without finishing cleanup; remove ${NAMES.container} and ${NAMES.network} with docker rm -f / docker network rm`,
    )
    process.exit(code)
  }
  stoppingSince = Date.now()
  await viewer?.stop()
  if (ownsContainer) {
    console.log(
      `Removing ${NAMES.container}${options.keepData ? '' : ' and its data'}…`,
    )
    await stopContainer(runCommand, launchId, options.keepData).catch((error) =>
      console.error(String(error)),
    )
  }
  process.exit(code)
}
