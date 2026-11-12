import { describe, expect, test } from 'bun:test'
import { launchOptionsFrom } from './config'
import {
  type CheckoutImage,
  type CommandResult,
  createNetwork,
  imageTag,
  instanceNames,
  planImage,
  type Runner,
  removeLeftovers,
  retireImages,
  type StartState,
  settleImages,
  startContainer,
  stopContainer,
  TRACE_VALUE_BYTES,
} from './neo-container'

const ok = (stdout = ''): CommandResult => ({ code: 0, stdout, stderr: '' })
const missing: CommandResult = { code: 1, stdout: '', stderr: 'No such object' }

/** A fake `docker` that answers from a function and records every call. */
function fakeDocker(answer: (args: string[]) => CommandResult) {
  const calls: string[][] = []
  const run: Runner = async (command, args) => {
    calls.push([command, ...args])
    return answer(args)
  }
  return { run, calls }
}

/**
 * A Docker holding the viewer's three names with these labels (`undefined`: no such object).
 * Answers `inspect` for the label asked, and records removals.
 */
function dockerWith(
  objects: Partial<
    Record<'container' | 'network' | 'volume', Record<string, string>>
  >,
) {
  return fakeDocker((args) => {
    const [kind, verb] = args
    if (
      verb === 'inspect' &&
      (kind === 'container' || kind === 'network' || kind === 'volume')
    ) {
      const labels = objects[kind]
      if (!labels) return missing
      const key = /"([^"]+)"/.exec(args[3] ?? '')?.[1] ?? ''
      // A container's or network's id, a volume's name; then the label asked for.
      const ref = kind === 'volume' ? NAMES.volume : `${kind}-id`
      return ok(`${ref}|${labels[key] ?? '<no value>'}\n`)
    }
    return ok()
  })
}

const NAMES = instanceNames('t1')
const ours = {
  'com.browseros.neo-viewer': '1',
  'com.browseros.neo-viewer.instance': 't1',
  'com.browseros.neo-viewer.launch': 'launch-a',
}
const removals = (calls: string[][]) =>
  calls.filter((call) => call.includes('rm') || call.includes('stop'))

describe('the image for this checkout', () => {
  const wanted = (overrides: Partial<CheckoutImage>): CheckoutImage => ({
    tag: 'neo-sdk:0.52.1.2-d6b008abb-arm64',
    cleanTag: 'neo-sdk:0.52.1.2-d6b008abb-arm64',
    fingerprint: null,
    buildInputsChanged: false,
    ...overrides,
  })
  const dirty = {
    tag: 'neo-sdk:0.52.1.2-d6b008abb-dirty-arm64',
    fingerprint: 'f1',
    buildInputsChanged: true,
  }

  test("is build.sh's tag: browser, 9-character commit, -dirty with changes, arch", () => {
    const parts = {
      browserVersion: '0.52.1.2',
      commit: 'd6b008abb633ed4d',
      arch: 'arm64',
    }
    expect(imageTag({ ...parts, dirty: false })).toBe(
      'neo-sdk:0.52.1.2-d6b008abb-arm64',
    )
    expect(imageTag({ ...parts, dirty: true })).toBe(
      'neo-sdk:0.52.1.2-d6b008abb-dirty-arm64',
    )
  })

  test('a clean checkout reuses its image when it is on the machine, and builds it when not', () => {
    expect(
      planImage(
        wanted({}),
        { tag: 'sha256:a', cleanTag: 'sha256:a' },
        { lastBuild: null },
      ),
    ).toEqual({
      kind: 'reuse',
      tag: wanted({}).tag,
    })
    expect(
      planImage(wanted({}), { tag: null, cleanTag: null }, { lastBuild: null }),
    ).toMatchObject({ kind: 'build' })
  })

  test("changes that touch no build input (this tool, say) reuse the commit's image", () => {
    const toolOnly = wanted({ tag: dirty.tag, fingerprint: 'empty' })
    expect(
      planImage(
        toolOnly,
        { tag: null, cleanTag: 'sha256:a' },
        { lastBuild: null },
      ),
    ).toEqual({
      kind: 'reuse',
      tag: toolOnly.cleanTag,
    })
    // Without the commit's image it is built, under the -dirty tag build.sh gives it.
    expect(
      planImage(toolOnly, { tag: null, cleanTag: null }, { lastBuild: null }),
    ).toMatchObject({
      kind: 'build',
      tag: dirty.tag,
    })
  })

  test('changed build inputs rebuild unless the image is the one built from the same changes', () => {
    const built = { tag: dirty.tag, id: 'sha256:a', fingerprint: 'f1' }
    const present = { tag: 'sha256:a', cleanTag: 'sha256:clean' }
    expect(planImage(wanted(dirty), present, { lastBuild: built })).toEqual({
      kind: 'reuse',
      tag: dirty.tag,
    })
    expect(
      planImage(wanted({ ...dirty, fingerprint: 'f2' }), present, {
        lastBuild: built,
      }),
    ).toMatchObject({
      kind: 'build',
    })
    // Someone else built this -dirty tag: its contents are unknown.
    expect(
      planImage(
        wanted(dirty),
        { tag: 'sha256:b', cleanTag: null },
        { lastBuild: built },
      ),
    ).toMatchObject({
      kind: 'build',
    })
    expect(
      planImage(wanted(dirty), present, { lastBuild: null }),
    ).toMatchObject({ kind: 'build' })
  })
})

describe('instances', () => {
  test('each instance has its own names', () => {
    expect(instanceNames('default')).toEqual({
      instance: 'default',
      container: 'neo-viewer-default',
      network: 'neo-viewer-default-net',
      volume: 'neo-viewer-default-data',
    })
    expect(instanceNames('vp-a').container).not.toBe(
      instanceNames('vp-b').container,
    )
  })

  test("a start looks only at its own instance's names, and refuses one labelled for another", async () => {
    const docker = dockerWith({ container: ours, network: ours, volume: ours })
    await removeLeftovers(docker.run, instanceNames('t1'), false)
    const inspected = new Set(
      docker.calls
        .filter((call) => call[2] === 'inspect')
        .map((call) => call.at(-1)),
    )
    expect(
      [...inspected].every(
        (name) => name?.startsWith('neo-viewer-t1') || name?.endsWith('-id'),
      ),
    ).toBe(true)
    const other = dockerWith({
      container: { ...ours, 'com.browseros.neo-viewer.instance': 't2' },
    })
    await expect(
      removeLeftovers(other.run, instanceNames('t1'), false),
    ).rejects.toThrow('did not create it')
    expect(removals(other.calls)).toEqual([])
  })

  test('--name scopes the state folder; a bad name is refused', () => {
    expect(
      launchOptionsFrom(['--name', 'vp-a'], { XDG_STATE_HOME: '/state' }),
    ).toMatchObject({
      instance: 'vp-a',
      dataDir: '/state/neo-viewer/vp-a',
    })
    expect(
      launchOptionsFrom([], {
        NEO_VIEWER_INSTANCE: 'ci',
        XDG_STATE_HOME: '/state',
      }).dataDir,
    ).toBe('/state/neo-viewer/ci')
    expect(launchOptionsFrom([], { XDG_STATE_HOME: '/state' }).instance).toBe(
      'default',
    )
    expect(() => launchOptionsFrom(['--name', 'Bad Name'], {})).toThrow(
      'bad --name',
    )
  })
})

describe('cleanup touches only what the viewer created', () => {
  test("a start removes the last start's labelled container, network and volume", async () => {
    const docker = dockerWith({ container: ours, network: ours, volume: ours })
    const removed = await removeLeftovers(docker.run, NAMES, false)
    expect(removed).toEqual([
      `container ${NAMES.container}`,
      `network ${NAMES.network}`,
      `volume ${NAMES.volume}`,
    ])
    expect(removals(docker.calls)).toEqual([
      ['docker', 'rm', '-f', 'container-id'],
      ['docker', 'network', 'rm', 'network-id'],
      ['docker', 'volume', 'rm', NAMES.volume],
    ])
  })

  test('with --keep-data the volume stays, the container gets its graceful stop first', async () => {
    const docker = dockerWith({ container: ours, volume: ours })
    expect(await removeLeftovers(docker.run, NAMES, true)).toEqual([
      `container ${NAMES.container}`,
    ])
    expect(removals(docker.calls)).toEqual([
      ['docker', 'stop', '--time', '25', 'container-id'],
      ['docker', 'rm', '-f', 'container-id'],
    ])
  })

  test('a same-named object without the label stops the start, even a volume --keep-data would keep', async () => {
    for (const keepData of [false, true]) {
      const docker = dockerWith({ volume: { owner: 'someone else' } })
      await expect(
        removeLeftovers(docker.run, NAMES, keepData),
      ).rejects.toThrow('did not create it')
      expect(removals(docker.calls)).toEqual([])
    }
  })

  test("Ctrl-C removes this launch's objects, never a later launch's or someone else's", async () => {
    const mine = dockerWith({ container: ours, network: ours, volume: ours })
    await stopContainer(mine.run, NAMES, 'launch-a', false)
    expect(removals(mine.calls)).toEqual([
      ['docker', 'rm', '-f', 'container-id'],
      ['docker', 'network', 'rm', 'network-id'],
      ['docker', 'volume', 'rm', NAMES.volume],
    ])
    const later = { ...ours, 'com.browseros.neo-viewer.launch': 'launch-b' }
    const replaced = dockerWith({
      container: later,
      network: later,
      volume: later,
    })
    await stopContainer(replaced.run, NAMES, 'launch-a', false)
    const foreign = dockerWith({ container: { owner: 'someone else' } })
    await stopContainer(foreign.run, NAMES, 'launch-a', true)
    expect([...removals(replaced.calls), ...removals(foreign.calls)]).toEqual(
      [],
    )
  })

  test('an image this instance replaced stays while any container or another instance holds it, and goes once nothing does', async () => {
    const image = {
      tag: 'neo-sdk:0.52.1.2-aaaaaaaaa-arm64',
      id: 'sha256:0123456789abcdef',
    }
    let containers = 'neo-viewer-default\n'
    const docker = fakeDocker((args) => {
      if (args[0] === 'image' && args[1] === 'inspect')
        return ok(`${image.id}\n`)
      if (args[0] === 'ps') return ok(containers)
      return ok()
    })
    const other = { lastBuild: { ...image, fingerprint: null } }
    // Another instance's container runs it (running or stopped: `ps -a`).
    expect(await retireImages(docker.run, [image], [])).toEqual({
      removed: [],
      kept: [{ ...image, reason: 'used by container neo-viewer-default' }],
    })
    // No container, but another instance's next start would reuse it.
    containers = ''
    expect(
      (await retireImages(docker.run, [image], [other])).kept[0]?.reason,
    ).toBe("another instance's last build")
    // Nothing holds it: removed, by a plain rm, never forced.
    expect(
      await retireImages(docker.run, [image], [{ lastBuild: null }]),
    ).toEqual({ removed: [image], kept: [] })
    const removals = docker.calls.filter(
      (call) => call[1] === 'image' && call[2] === 'rm',
    )
    expect(removals).toEqual([['docker', 'image', 'rm', image.id]])
  })

  test('two instances on one image: a later start of one keeps it while the other uses it, and removes it once nothing does', async () => {
    const x = {
      tag: 'neo-sdk:0.52.1.2-aaaaaaaaa-arm64',
      id: 'sha256:aaaaaaaaaaaaaaaa',
    }
    const y = {
      tag: 'neo-sdk:0.52.1.2-bbbbbbbbb-arm64',
      id: 'sha256:bbbbbbbbbbbbbbbb',
    }
    const images = new Set([x.id, y.id])
    // Which containers, running or stopped, use each image.
    const users = new Map<string, string[]>([[x.id, ['neo-viewer-b']]])
    const docker = fakeDocker((args) => {
      if (args[0] === 'image' && args[1] === 'inspect') {
        const ref = args.at(-1) ?? ''
        return images.has(ref)
          ? ok(`${ref}\n`)
          : { code: 1, stdout: '', stderr: 'No such image' }
      }
      if (args[0] === 'ps') {
        const ancestor =
          args
            .find((arg) => arg.startsWith('ancestor='))
            ?.slice('ancestor='.length) ?? ''
        return ok(
          (users.get(ancestor) ?? []).map((name) => `${name}\n`).join(''),
        )
      }
      if (args[0] === 'image' && args[1] === 'rm') {
        images.delete(args.at(-1) ?? '')
        return ok()
      }
      return ok()
    })
    // Instance a built x; instance b reused it (a reuse records no build) and its container runs it.
    let a: StartState = { lastBuild: { ...x, fingerprint: null } }
    const b: StartState = { lastBuild: null, kept: [] }

    // a moves to a new commit and builds y: x stays, for b's container.
    const first = await settleImages(
      docker.run,
      a,
      { built: { ...y, fingerprint: null }, running: y.id },
      [b],
    )
    expect(first.kept).toEqual([
      { ...x, reason: 'used by container neo-viewer-b' },
    ])
    expect(first.removed).toEqual([])
    expect(images.has(x.id)).toBe(true)
    a = first.state
    expect(a).toEqual({ lastBuild: { ...y, fingerprint: null }, kept: [x] })

    // b stops (its container is removed); a's next start reuses y and removes x.
    users.clear()
    const second = await settleImages(
      docker.run,
      a,
      { built: null, running: y.id },
      [b],
    )
    expect(second.removed).toEqual([x])
    expect(second.state).toEqual({
      lastBuild: { ...y, fingerprint: null },
      kept: [],
    })
    expect(images.has(x.id)).toBe(false)
    // Never forced, and never the image a start runs.
    const removals = docker.calls.filter(
      (call) => call[1] === 'image' && call[2] === 'rm',
    )
    expect(removals).toEqual([['docker', 'image', 'rm', x.id]])
  })

  test('two instances that both kept one image do not wait on each other; an unanswered docker ps keeps it', async () => {
    const x = { tag: 'neo-sdk:x-arm64', id: 'sha256:aaaaaaaaaaaaaaaa' }
    let psFails = true
    const docker = fakeDocker((args) => {
      if (args[0] === 'image' && args[1] === 'inspect') return ok(`${x.id}\n`)
      if (args[0] === 'ps')
        return psFails
          ? {
              code: 1,
              stdout: '',
              stderr: 'Cannot connect to the Docker daemon\n',
            }
          : ok()
      return ok()
    })
    const other: StartState = { lastBuild: null, kept: [x] }
    const mine: StartState = { lastBuild: null, kept: [x] }
    const unknown = await settleImages(
      docker.run,
      mine,
      { built: null, running: 'sha256:cccccccccccccccc' },
      [other],
    )
    expect(unknown.kept[0]?.reason).toBe(
      'docker ps failed (Cannot connect to the Docker daemon)',
    )
    psFails = false
    expect(
      (
        await settleImages(
          docker.run,
          mine,
          { built: null, running: 'sha256:cccccccccccccccc' },
          [other],
        )
      ).removed,
    ).toEqual([x])
  })

  test('a Docker that does not answer an inspect keeps the image on the list; one that says it is gone drops it', async () => {
    const x = { tag: 'neo-sdk:x-arm64', id: 'sha256:aaaaaaaaaaaaaaaa' }
    let inspect: CommandResult = {
      code: 1,
      stdout: '',
      stderr: 'Cannot connect to the Docker daemon\n',
    }
    const docker = fakeDocker((args) =>
      args[0] === 'image' && args[1] === 'inspect' ? inspect : ok(),
    )
    const state: StartState = { lastBuild: null, kept: [x] }
    const unanswered = await settleImages(
      docker.run,
      state,
      { built: null, running: 'sha256:cccccccccccccccc' },
      [],
    )
    expect(unanswered.state.kept).toEqual([x])
    expect(unanswered.kept[0]?.reason).toBe(
      'docker image inspect failed (Cannot connect to the Docker daemon)',
    )
    inspect = {
      code: 1,
      stdout: '',
      stderr:
        'Error response from daemon: No such image: sha256:aaaaaaaaaaaaaaaa\n',
    }
    const gone = await settleImages(
      docker.run,
      state,
      { built: null, running: 'sha256:cccccccccccccccc' },
      [],
    )
    expect(gone.state.kept).toEqual([])
    expect(docker.calls.some((call) => call[2] === 'rm')).toBe(false)
  })

  test('the image a start runs is never retired, even when an earlier start kept it', async () => {
    const x = { tag: 'neo-sdk:x-arm64', id: 'sha256:aaaaaaaaaaaaaaaa' }
    const y = { tag: 'neo-sdk:y-arm64', id: 'sha256:bbbbbbbbbbbbbbbb' }
    const docker = fakeDocker(() => ok(`${x.id}\n`))
    const settled = await settleImages(
      docker.run,
      { lastBuild: { ...y, fingerprint: null }, kept: [x] },
      { built: null, running: x.id },
      [],
    )
    // Not removed, and still recorded: once this instance moves off it, a later start removes it.
    expect(settled).toEqual({
      state: { lastBuild: { ...y, fingerprint: null }, kept: [x] },
      removed: [],
      kept: [],
    })
    expect(docker.calls.some((call) => call[2] === 'rm')).toBe(false)
  })

  test("Docker's own refusal keeps the image too, with its reason", async () => {
    const image = { tag: 'neo-sdk:x-arm64', id: 'sha256:fedcba9876543210' }
    const docker = fakeDocker((args) => {
      if (args[0] === 'image' && args[1] === 'inspect')
        return ok(`${image.id}\n`)
      if (args[0] === 'image' && args[1] === 'rm') {
        return {
          code: 1,
          stdout: '',
          stderr:
            'Error response from daemon: conflict: unable to delete (must be forced)\n',
        }
      }
      return ok()
    })
    expect(
      (await retireImages(docker.run, [image], [])).kept[0]?.reason,
    ).toContain('must be forced')
  })
})

test("the container keeps a call's arguments and result whole up to Neo's reply limit", async () => {
  // docker run fails here, after its arguments are recorded: they are what this checks.
  const docker = fakeDocker((args) =>
    args[0] === 'run'
      ? { code: 1, stdout: '', stderr: 'stopped by the test' }
      : ok(),
  )
  await expect(
    startContainer(docker.run, {
      names: NAMES,
      image: 'neo-sdk:test',
      tokenFile: '/tmp/token',
      launch: 'launch-a',
    }),
  ).rejects.toThrow()
  const run = docker.calls.find((call) => call[1] === 'run') ?? []
  expect(TRACE_VALUE_BYTES).toBe(256 * 1024)
  expect(run).toContain(`CLAW_TRACE_VALUE_BYTES=${TRACE_VALUE_BYTES}`)
  expect(
    run[run.indexOf(`CLAW_TRACE_VALUE_BYTES=${TRACE_VALUE_BYTES}`) - 1],
  ).toBe('-e')
})

test('a machine whose Docker address pools are used up still gets a network, on an explicit subnet', async () => {
  const docker = fakeDocker((args) => {
    if (!args.includes('--subnet')) {
      return {
        code: 1,
        stdout: '',
        stderr: 'all predefined address pools have been fully subnetted',
      }
    }
    return args.includes('10.213.0.0/24')
      ? {
          code: 1,
          stdout: '',
          stderr: 'Pool overlaps with other one on this address space',
        }
      : ok()
  })
  await createNetwork(docker.run, NAMES, [
    '--label',
    'com.browseros.neo-viewer=1',
  ])
  expect(docker.calls.map((call) => call.at(-2))).toEqual([
    'com.browseros.neo-viewer=1',
    '10.213.0.0/24',
    '10.213.1.0/24',
  ])
})

test('launch options: no arguments starts a container; --upstream starts none; the data folder is absolute', () => {
  expect(launchOptionsFrom([], {})).toMatchObject({
    upstream: null,
    keepData: false,
    build: true,
    image: null,
    port: 7310,
  })
  expect(
    launchOptionsFrom(
      ['--keep-data', '--no-build', '--image', 'neo-sdk:t'],
      {},
    ),
  ).toMatchObject({
    keepData: true,
    build: false,
    image: 'neo-sdk:t',
  })
  expect(
    launchOptionsFrom(['--upstream', 'http://127.0.0.1:19010/mcp'], {}).upstream
      ?.href,
  ).toBe('http://127.0.0.1:19010/mcp')
  expect(launchOptionsFrom(['--data-dir', 'state'], {}).dataDir).toBe(
    `${process.cwd()}/state`,
  )
})
