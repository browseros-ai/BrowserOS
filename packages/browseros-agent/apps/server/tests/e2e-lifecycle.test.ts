import { describe, expect, it, spyOn } from 'bun:test'
import {
  access,
  chmod,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  isolatedEnvironment,
  OwnedProcess,
  requireBrowserBinary,
  reservePort,
  waitUntil,
  withE2e,
} from './__helpers__/e2e'

async function withTemp(run: (root: string) => Promise<void>) {
  const root = await mkdtemp(join(tmpdir(), 'owned-e2e-lifecycle-'))
  try {
    await run(root)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

describe('owned E2E lifecycle (no browser required)', () => {
  it('requires an explicit executable instead of silently skipping', async () => {
    await expect(requireBrowserBinary('')).rejects.toThrow('BROWSEROS_BINARY')
    await expect(
      requireBrowserBinary('/does-not-exist/browseros'),
    ).rejects.toThrow('not executable')
  })

  it('does not inherit secrets or application config overrides', async () => {
    await withTemp(async (root) => {
      const env = await isolatedEnvironment(root)
      expect(Object.keys(env).sort()).toEqual([
        'BROWSEROS_DIR',
        'HOME',
        'LANG',
        'NODE_ENV',
        'PATH',
        'TMPDIR',
        'XDG_CACHE_HOME',
        'XDG_CONFIG_HOME',
        'XDG_DATA_HOME',
      ])
      expect(env.HOME).toBe(join(root, 'home'))
      expect(env.BROWSEROS_DIR).toBe(join(root, 'browseros'))
    })
  })

  it('reports early exit with bounded child diagnostics', async () => {
    await withTemp(async (root) => {
      const child = new OwnedProcess(
        process.execPath,
        [
          '-e',
          "console.error('x'.repeat(20000) + 'early-exit-marker'); process.exit(7)",
        ],
        await isolatedEnvironment(root),
      )
      try {
        await expect(
          waitUntil('fixture', [child], async () => false, 3_000),
        ).rejects.toThrow('early-exit-marker')
        expect(child.diagnostics().length).toBeLessThanOrEqual(16_384)
        expect(child.child.exitCode).toBe(7)
      } finally {
        await child.stop()
      }
    })
  })

  it('times out and stops only its owned process, preserving another listener', async () => {
    await withTemp(async (root) => {
      const listener = await reservePort()
      const second = await reservePort()
      const child = new OwnedProcess(
        process.execPath,
        ['-e', 'setInterval(() => {}, 1000)'],
        await isolatedEnvironment(root),
      )
      try {
        expect(second.port).not.toBe(listener.port)
        await expect(
          waitUntil('never-ready fixture', [child], async () => false, 100),
        ).rejects.toThrow('Timed out')
        await child.stop()
        expect(
          child.child.exitCode !== null || child.child.signalCode !== null,
        ).toBe(true)
        // A successful connection proves cleanup did not kill/close an unrelated owner.
        const connection = await Bun.connect({
          hostname: '127.0.0.1',
          port: listener.port,
          socket: { data() {} },
        })
        connection.end()
      } finally {
        await child.stop()
        await second.release()
        await listener.release()
      }
    })
  })

  it('escalates the owned group after its leader exits and shares stop completion', async () => {
    await withTemp(async (root) => {
      const ready = join(root, 'descendant-ready')
      const leader = join(root, 'leader.ts')
      const descendant = join(root, 'descendant.ts')
      await writeFile(
        descendant,
        `
        import { writeFileSync } from 'node:fs'
        process.on('SIGTERM', () => {})
        writeFileSync(${JSON.stringify(ready)}, String(process.pid))
        setTimeout(() => process.exit(0), 6000) // finite fixture watchdog
        setInterval(() => {}, 100)
      `,
      )
      await writeFile(
        leader,
        `
        import { spawn } from 'node:child_process'
        spawn(process.execPath, [${JSON.stringify(descendant)}], { stdio: 'inherit' })
        process.on('SIGTERM', () => process.exit(0))
        setTimeout(() => process.exit(0), 6000)
      `,
      )
      const child = new OwnedProcess(
        process.execPath,
        [leader],
        await isolatedEnvironment(root),
      )
      let descendantPid: number | undefined
      const running = async () => {
        if (descendantPid === undefined) return false
        if (process.platform === 'linux') {
          try {
            // A terminated orphan can remain a zombie until the container's init reaps it.
            const stat = await readFile(`/proc/${descendantPid}/stat`, 'utf8')
            return stat.slice(stat.lastIndexOf(')') + 2).split(' ')[0] !== 'Z'
          } catch (error) {
            if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false
            throw error
          }
        }
        try {
          process.kill(descendantPid, 0)
          return true
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code === 'ESRCH') return false
          throw error
        }
      }
      try {
        await waitUntil(
          'descendant signal handler ready',
          [child],
          async () => {
            try {
              descendantPid = Number(await readFile(ready, 'utf8'))
              return Number.isInteger(descendantPid) && descendantPid > 0
            } catch {
              return false
            }
          },
          2_000,
        )
        expect(await running()).toBe(true)
        const first = child.stop()
        expect(child.stop()).toBe(first)
        await first
        expect(child.child.exitCode).toBe(0)
        await waitUntil(
          'TERM-ignoring descendant terminated',
          [],
          async () => !(await running()),
          1_000,
        )
        expect(await running()).toBe(false)
        expect(child.stop()).toBe(first)
      } finally {
        await child.stop()
        // Even a regressed implementation cannot leave this fixture alive forever.
        // The watchdog is in the descendant itself, not in the test runner.
        await waitUntil(
          'fixture watchdog cleanup',
          [],
          async () => !(await running()),
          7_000,
        )
      }
    })
  }, 12_000)

  it('reports signalling errors even when final group escalation succeeds', async () => {
    await withTemp(async (root) => {
      const child = new OwnedProcess(
        process.execPath,
        [
          '-e',
          "console.log('signal-fixture-ready'); setTimeout(() => process.exit(0), 6000)",
        ],
        await isolatedEnvironment(root),
      )
      const originalKill = process.kill.bind(process)
      await waitUntil(
        'signal fixture ready',
        [child],
        async () => child.diagnostics().includes('signal-fixture-ready'),
        2_000,
      ).catch(async (error) => {
        await child.stop()
        throw error
      })
      const kill = spyOn(process, 'kill').mockImplementation((pid, signal) => {
        if (pid === -(child.child.pid ?? 0) && signal === 'SIGTERM') {
          throw Object.assign(new Error('fixture EPERM'), { code: 'EPERM' })
        }
        return originalKill(pid, signal)
      })
      try {
        await expect(child.stop()).rejects.toThrow(
          'Owned process-group teardown failed',
        )
        expect(child.child.signalCode).toBe('SIGKILL')
        await expect(child.stop()).rejects.toThrow(
          'Owned process-group teardown failed',
        )
      } finally {
        kill.mockRestore()
        await child.stop().catch(() => {})
        expect(
          child.child.exitCode !== null || child.child.signalCode !== null,
        ).toBe(true)
      }
    })
  })

  it('cleans a partial browser startup without calling the test body', async () => {
    await withTemp(async (root) => {
      const executable = join(root, 'exits-before-cdp')
      const record = join(root, 'profile-path')
      await writeFile(
        executable,
        `#!/bin/sh\nfor arg in "$@"; do\ncase "$arg" in --user-data-dir=*) printf '%s' "\${arg#--user-data-dir=}" > '${record}';; esac\ndone\necho partial-start-marker >&2\nexit 9\n`,
      )
      await chmod(executable, 0o700)
      let called = false
      await expect(
        withE2e(async () => {
          called = true
        }, executable),
      ).rejects.toThrow('partial-start-marker')
      expect(called).toBe(false)
      const profile = await readFile(record, 'utf8')
      expect(profile).toContain('browseros-owned-e2e-')
      await expect(access(profile)).rejects.toThrow()
    })
  })
})
