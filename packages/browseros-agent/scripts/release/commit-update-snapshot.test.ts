import { describe, expect, it } from 'bun:test'
import { spawnSync } from 'node:child_process'
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')
const script = resolve(import.meta.dir, 'commit-update-snapshot.sh')

function run(cwd: string, command: string[], env: Record<string, string> = {}) {
  const result = spawnSync(command[0], command.slice(1), {
    cwd,
    env: { ...process.env, ...env },
    encoding: 'utf8',
  })
  return {
    code: result.status ?? 1,
    stdout: result.stdout,
    stderr: result.stderr,
  }
}

function mustRun(cwd: string, command: string[]) {
  const result = run(cwd, command)
  expect(result.code, result.stderr || result.stdout).toBe(0)
  return result.stdout.trim()
}

function configureGit(dir: string) {
  mustRun(dir, ['git', 'config', 'user.name', 'Release Test'])
  mustRun(dir, ['git', 'config', 'user.email', 'release-test@example.com'])
}

function initFixture() {
  const root = mkdtempSync(join(tmpdir(), 'commit-update-snapshot-'))
  const remote = join(root, 'remote.git')
  const source = join(root, 'source')
  const competitor = join(root, 'competitor')

  mkdirSync(source)
  mustRun(root, ['git', 'init', '--bare', '--initial-branch=main', remote])
  mustRun(source, ['git', 'init', '--initial-branch=main'])
  configureGit(source)
  mkdirSync(join(source, 'updates/server'), { recursive: true })
  writeFileSync(
    join(source, 'updates/server/appcast-server.alpha.xml'),
    'server-old\n',
  )
  writeFileSync(
    join(source, 'updates/server/appcast-claw-server.alpha.xml'),
    'claw-old\n',
  )
  mustRun(source, ['git', 'add', 'updates'])
  mustRun(source, ['git', 'commit', '-m', 'initial snapshots'])
  mustRun(source, ['git', 'remote', 'add', 'origin', remote])
  mustRun(source, ['git', 'push', '-u', 'origin', 'main'])
  mustRun(root, ['git', 'clone', remote, competitor])
  configureGit(competitor)

  return { root, remote, source, competitor }
}

describe('commit-update-snapshot', () => {
  it('rebases and retries when an unrelated snapshot wins the first push', () => {
    const fixture = initFixture()
    try {
      writeFileSync(
        join(fixture.source, 'updates/server/appcast-server.alpha.xml'),
        'server-new\n',
      )
      writeFileSync(
        join(
          fixture.competitor,
          'updates/server/appcast-claw-server.alpha.xml',
        ),
        'claw-new\n',
      )
      mustRun(fixture.competitor, [
        'git',
        'add',
        'updates/server/appcast-claw-server.alpha.xml',
      ])
      mustRun(fixture.competitor, [
        'git',
        'commit',
        '-m',
        'snapshot competing claw feed',
      ])

      const realGit = mustRun(repoRoot, ['which', 'git'])
      const wrapperDir = join(fixture.root, 'bin')
      const wrapper = join(wrapperDir, 'git')
      const marker = join(fixture.root, 'raced')
      mkdirSync(wrapperDir)
      writeFileSync(
        wrapper,
        `#!/bin/sh
case " $* " in
  *" push "*)
    if [ ! -e "$SNAPSHOT_RACE_MARKER" ]; then
      : > "$SNAPSHOT_RACE_MARKER"
      "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_RACE_REPO" push origin HEAD:main || exit $?
    fi
    ;;
esac
exec "$SNAPSHOT_REAL_GIT" "$@"
`,
      )
      chmodSync(wrapper, 0o755)

      const result = run(
        fixture.source,
        [
          script,
          'updates/server/appcast-server.alpha.xml',
          'main',
          'snapshot BrowserOS server alpha 1.2.3',
        ],
        {
          PATH: `${wrapperDir}:${process.env.PATH}`,
          SNAPSHOT_RACE_MARKER: marker,
          SNAPSHOT_RACE_REPO: fixture.competitor,
          SNAPSHOT_REAL_GIT: realGit,
        },
      )

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(result.stdout).toContain('Snapshot push rejected; retrying')
      expect(
        mustRun(fixture.root, [
          'git',
          `--git-dir=${fixture.remote}`,
          'show',
          'main:updates/server/appcast-server.alpha.xml',
        ]),
      ).toBe('server-new')
      expect(
        mustRun(fixture.root, [
          'git',
          `--git-dir=${fixture.remote}`,
          'show',
          'main:updates/server/appcast-claw-server.alpha.xml',
        ]),
      ).toBe('claw-new')
    } finally {
      rmSync(fixture.root, { recursive: true, force: true })
    }
  })

  it('succeeds without a commit when the snapshot is already current', () => {
    const fixture = initFixture()
    try {
      const before = mustRun(fixture.remote, ['git', 'rev-parse', 'main'])
      const result = run(fixture.source, [
        script,
        'updates/server/appcast-server.alpha.xml',
        'main',
        'snapshot BrowserOS server alpha 1.2.3',
      ])

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(result.stdout).toContain('Snapshot already current')
      expect(mustRun(fixture.remote, ['git', 'rev-parse', 'main'])).toBe(before)
    } finally {
      rmSync(fixture.root, { recursive: true, force: true })
    }
  })

  it('rejects paths outside updates and missing snapshots', () => {
    const fixture = initFixture()
    try {
      const outside = run(fixture.source, [
        script,
        'README.md',
        'main',
        'invalid snapshot',
      ])
      const missing = run(fixture.source, [
        script,
        'updates/server/missing.xml',
        'main',
        'missing snapshot',
      ])

      expect(outside.code).not.toBe(0)
      expect(outside.stderr).toContain('must be under updates/')
      expect(missing.code).not.toBe(0)
      expect(missing.stderr).toContain('Snapshot does not exist')
    } finally {
      rmSync(fixture.root, { recursive: true, force: true })
    }
  })
})
