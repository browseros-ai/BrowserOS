import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import createExtensionUpdateApi from './api.js'

const ID = 'pjimfkbpehlcllblajnpfamdfjhhlgkc'
function fixture(factory = createExtensionUpdateApi) {
  const stored: Record<string, unknown> = {}
  let running = '0.2.28.0'
  let pending:
    | { manifest: { version: string }; delay_install_reason: number }
    | undefined
  const reloads: string[] = []
  const restoredFiles: string[][] = []
  const stoppedTabs: number[] = []
  const scheduled: Array<() => void> = []
  let flush: () => Promise<unknown> = async () => ({ persisted: true })
  let injectionFails = false
  let legacyDrain: () => Promise<{ persisted?: boolean }> = async () => ({
    persisted: true,
  })
  const chrome = {
    runtime: {
      id: ID,
      getManifest: () => ({
        version: running,
        content_scripts: [
          { matches: ['<all_urls>'], js: ['recorder.js'] },
          { matches: ['https://special.example/*'], js: ['restricted.js'] },
        ],
      }),
      reload: () => {
        reloads.push(running)
      },
      sendMessage: () => legacyDrain(),
      lastError: undefined,
    },
    storage: {
      local: {
        get: async (key: string) => ({ [key]: stored[key] }),
        set: async (items: Record<string, unknown>) => {
          Object.assign(stored, items)
        },
        remove: async (key: string) => {
          delete stored[key]
        },
      },
    },
    tabs: {
      query: async () => [{ id: 7 }],
      sendMessage: async (id: number) => {
        stoppedTabs.push(id)
        return flush()
      },
    },
    scripting: {
      executeScript: async (options: {
        files?: string[]
        func?: () => unknown
      }) => {
        if (injectionFails) throw new Error('Temporary injection failure')
        if (options.files) restoredFiles.push(options.files)
        return [
          {
            result:
              options.func?.name === 'flushLegacyRecorder'
                ? await options.func()
                : undefined,
          },
        ]
      },
    },
    browserOS: {
      getPref: (_name: string, callback: (pref: { value: unknown }) => void) =>
        callback({ value: { [ID]: { idle_install_info: pending } } }),
    },
  }
  return {
    api: () =>
      factory(chrome, {
        now: () => 1000,
        schedule: (fn: () => void) => scheduled.push(fn),
      }),
    stage(version = '0.2.29.0', reason = 2) {
      pending = { manifest: { version }, delay_install_reason: reason }
    },
    activate() {
      if (!pending) throw new Error('No staged update')
      running = pending.manifest.version
      pending = undefined
    },
    reloads,
    scheduled,
    restoredFiles,
    stoppedTabs,
    setFlush: (callback: () => Promise<unknown>) => {
      flush = callback
    },
    failInjection: (fails: boolean) => {
      injectionFails = fails
    },
    setLegacyDrain: (callback: () => Promise<{ persisted?: boolean }>) => {
      legacyDrain = callback
    },
  }
}

describe('extension update API', () => {
  it('also runs as the standalone factory injected into older workers over CDP', async () => {
    const source = readFileSync(new URL('./api.js', import.meta.url), 'utf8')
    const factory = new Function(
      `return (${source.replace('export default ', '')})`,
    )() as typeof createExtensionUpdateApi
    const f = fixture(factory)
    f.stage()
    expect(await f.api().applyPendingUpdate('0.2.29.0')).toEqual({
      scheduled: true,
    })
  })
  it('finds a staged update even if its notification was missed, and survives worker restart', async () => {
    const f = fixture()
    f.stage()
    expect(await f.api().getStatus()).toEqual({
      extensionId: ID,
      runningVersion: '0.2.28.0',
      pendingVersion: '0.2.29.0',
      detectedAt: 1000,
      attemptedVersion: null,
    })
    expect((await f.api().getStatus()).pendingVersion).toBe('0.2.29.0')
  })
  it('coalesces concurrent activation calls and persists the attempt across restart', async () => {
    const f = fixture()
    f.stage()
    const api = f.api()
    const results = await Promise.all([
      api.applyPendingUpdate('0.2.29.0'),
      api.applyPendingUpdate('0.2.29.0'),
    ])
    expect(results).toEqual([{ scheduled: true }, { scheduled: false }])
    expect(await f.api().applyPendingUpdate('0.2.29.0')).toEqual({
      scheduled: false,
    })
    expect(f.reloads).toEqual([])
    expect(f.stoppedTabs).toEqual([7])
    for (const fn of f.scheduled) fn()
    expect(f.reloads).toEqual(['0.2.28.0'])
  })

  it('waits for the recorder stop acknowledgment before scheduling reload', async () => {
    const f = fixture()
    f.stage()
    let finish: (value: { persisted: boolean }) => void = () => {}
    const pendingFlush = new Promise<{ persisted: boolean }>((resolve) => {
      finish = resolve
    })
    let stopping: () => void = () => {}
    const reachedStop = new Promise<void>((resolve) => {
      stopping = resolve
    })
    f.setFlush(() => {
      stopping()
      return pendingFlush
    })
    const activation = f.api().applyPendingUpdate('0.2.29.0')
    await reachedStop
    expect(f.scheduled).toHaveLength(0)
    finish({ persisted: true })
    expect(await activation).toEqual({ scheduled: true })
    expect(f.scheduled).toHaveLength(1)
  })

  it('never reloads for an event alone, stale requested version, or a different install gate', async () => {
    const f = fixture()
    const api = f.api()
    await api.recordUpdate('0.2.29.0')
    expect(await api.applyPendingUpdate('0.2.29.0')).toEqual({
      scheduled: false,
    })
    f.stage('0.2.30.0')
    expect(await api.applyPendingUpdate('0.2.29.0')).toEqual({
      scheduled: false,
    })
    f.stage('0.2.30.0', 3)
    expect(await api.applyPendingUpdate('0.2.30.0')).toEqual({
      scheduled: false,
    })
    expect(f.scheduled).toHaveLength(0)
  })

  it('resumes recorders and defers reload when their flush fails', async () => {
    const f = fixture()
    f.stage()
    f.setFlush(async () => ({ persisted: false }))
    const api = f.api()
    await expect(api.applyPendingUpdate('0.2.29.0')).rejects.toThrow(
      'not persisted',
    )
    expect(f.scheduled).toHaveLength(0)
    expect(f.restoredFiles).toEqual([['recorder.js']])
    expect((await api.getStatus()).attemptedVersion).toBeNull()
    f.setFlush(async () => ({ persisted: true }))
    expect(await api.applyPendingUpdate('0.2.29.0')).toEqual({
      scheduled: true,
    })
  })

  it('uses the durable relay barrier for legacy recorders and requires its acknowledgment', async () => {
    const f = fixture()
    f.stage()
    f.setFlush(async () => undefined)
    f.setLegacyDrain(async () => ({ persisted: false }))
    const api = f.api()
    await expect(api.applyPendingUpdate('0.2.29.0')).rejects.toThrow(
      'Legacy recorder flush was not persisted',
    )
    expect(f.scheduled).toHaveLength(0)
    f.setLegacyDrain(async () => ({ persisted: true }))
    expect(await api.applyPendingUpdate('0.2.29.0')).toEqual({
      scheduled: true,
    })
  })

  it('retains recovery state after a transient injection failure and retries', async () => {
    const f = fixture()
    f.stage()
    await f.api().applyPendingUpdate('0.2.29.0')
    f.activate()
    f.failInjection(true)
    const replacement = f.api()
    await expect(replacement.getStatus()).rejects.toThrow(
      'Temporary injection failure',
    )
    f.failInjection(false)
    expect((await replacement.getStatus()).pendingVersion).toBeNull()
    expect(f.restoredFiles).toEqual([['recorder.js']])
  })

  it('clears pending state after activation and permits a later update', async () => {
    const f = fixture()
    f.stage()
    await f.api().applyPendingUpdate('0.2.29.0')
    f.activate()
    const replacement = f.api()
    const [status] = await Promise.all([
      replacement.getStatus(),
      replacement.restoreContentScripts(),
    ])
    expect(status).toEqual({
      extensionId: ID,
      runningVersion: '0.2.29.0',
      pendingVersion: null,
      detectedAt: null,
      attemptedVersion: null,
    })
    expect(f.restoredFiles).toEqual([['recorder.js']])
    f.stage('0.2.30.0')
    expect(await f.api().applyPendingUpdate('0.2.30.0')).toEqual({
      scheduled: true,
    })
  })

  it('compares Chromium versions numerically and rejects malformed or older versions', async () => {
    const f = fixture()
    for (const version of ['0.2.9.0', '0.2.28', '0.2.29oops', '0.2.99999.0']) {
      f.stage(version)
      expect((await f.api().getStatus()).pendingVersion).toBeNull()
    }
    f.stage('0.2.100.0')
    expect((await f.api().getStatus()).pendingVersion).toBe('0.2.100.0')
  })
})
