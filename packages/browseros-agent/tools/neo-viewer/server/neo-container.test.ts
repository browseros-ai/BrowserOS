import { describe, expect, test } from 'bun:test'
import { launchOptionsFrom } from './config'
import {
  type CheckoutImage,
  type CommandResult,
  createNetwork,
  imageTag,
  NAMES,
  planImage,
  type Runner,
  removeLeftovers,
  removeReplacedImage,
  stopContainer,
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
      return ok(`${labels[key] ?? '<no value>'}\n`)
    }
    return ok()
  })
}

const ours = {
  'com.browseros.neo-viewer': '1',
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

describe('cleanup touches only what the viewer created', () => {
  test("a start removes the last start's labelled container, network and volume", async () => {
    const docker = dockerWith({ container: ours, network: ours, volume: ours })
    const removed = await removeLeftovers(docker.run, false)
    expect(removed).toEqual([
      `container ${NAMES.container}`,
      `network ${NAMES.network}`,
      `volume ${NAMES.volume}`,
    ])
    expect(removals(docker.calls)).toEqual([
      ['docker', 'rm', '-f', NAMES.container],
      ['docker', 'network', 'rm', NAMES.network],
      ['docker', 'volume', 'rm', NAMES.volume],
    ])
  })

  test('with --keep-data the volume stays, the container gets its graceful stop first', async () => {
    const docker = dockerWith({ container: ours, volume: ours })
    expect(await removeLeftovers(docker.run, true)).toEqual([
      `container ${NAMES.container}`,
    ])
    expect(removals(docker.calls)).toEqual([
      ['docker', 'stop', '--time', '25', NAMES.container],
      ['docker', 'rm', '-f', NAMES.container],
    ])
  })

  test('a same-named object without the label stops the start, even a volume --keep-data would keep', async () => {
    for (const keepData of [false, true]) {
      const docker = dockerWith({ volume: { owner: 'someone else' } })
      await expect(removeLeftovers(docker.run, keepData)).rejects.toThrow(
        'the viewer did not create it',
      )
      expect(removals(docker.calls)).toEqual([])
    }
  })

  test("Ctrl-C removes this launch's objects, never a later launch's or someone else's", async () => {
    const mine = dockerWith({ container: ours, network: ours, volume: ours })
    await stopContainer(mine.run, 'launch-a', false)
    expect(removals(mine.calls)).toEqual([
      ['docker', 'rm', '-f', NAMES.container],
      ['docker', 'network', 'rm', NAMES.network],
      ['docker', 'volume', 'rm', NAMES.volume],
    ])
    const later = { ...ours, 'com.browseros.neo-viewer.launch': 'launch-b' }
    const replaced = dockerWith({
      container: later,
      network: later,
      volume: later,
    })
    await stopContainer(replaced.run, 'launch-a', false)
    const foreign = dockerWith({ container: { owner: 'someone else' } })
    await stopContainer(foreign.run, 'launch-a', true)
    expect([...removals(replaced.calls), ...removals(foreign.calls)]).toEqual(
      [],
    )
  })

  test('removes only the image its own last build recorded, once a new build replaced it', async () => {
    const docker = fakeDocker(() => ok())
    const previous = {
      tag: 'neo-sdk:old-arm64',
      id: 'sha256:0123456789abcdef',
      fingerprint: null,
    }
    expect(await removeReplacedImage(docker.run, null, 'sha256:new')).toBeNull()
    expect(
      await removeReplacedImage(docker.run, previous, previous.id),
    ).toBeNull()
    expect(
      await removeReplacedImage(docker.run, previous, 'sha256:new'),
    ).toContain('neo-sdk:old-arm64')
    expect(docker.calls).toEqual([['docker', 'image', 'rm', previous.id]])
  })
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
  await createNetwork(docker.run, ['--label', 'com.browseros.neo-viewer=1'])
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
