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
  const wrapperDir = join(root, 'bin')
  const prHead = join(root, 'pr-head')
  const prHeadSha = join(root, 'pr-head-sha')

  mkdirSync(source)
  mkdirSync(wrapperDir)
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

  const realGit = mustRun(repoRoot, ['which', 'git'])
  const gh = join(wrapperDir, 'gh')
  writeFileSync(
    gh,
    [
      '#!/bin/sh',
      'set -eu',
      'command_name="$1"',
      'subcommand="$2"',
      'shift 2',
      'case "$command_name:$subcommand" in',
      '  pr:create)',
      '    head=""',
      '    while [ "$#" -gt 0 ]; do',
      '      case "$1" in',
      '        --head) head="$2"; shift 2 ;;',
      '        *) shift ;;',
      '      esac',
      '    done',
      '    printf "%s\\n" "$head" > "$SNAPSHOT_PR_HEAD_FILE"',
      '    "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" ls-remote --heads origin "$head" | cut -f1 > "$SNAPSHOT_PR_HEAD_SHA_FILE"',
      '    echo "https://example.test/pull/1"',
      '    ;;',
      '  pr:merge)',
      `    if [ -n "\${SNAPSHOT_MERGE_FAILURE_FILE:-}" ] && [ ! -e "$SNAPSHOT_MERGE_FAILURE_FILE" ]; then`,
      '      : > "$SNAPSHOT_MERGE_FAILURE_FILE"',
      '      exit 1',
      '    fi',
      '    head="$(cat "$SNAPSHOT_PR_HEAD_FILE")"',
      '    expected_head=""',
      '    while [ "$#" -gt 0 ]; do',
      '      case "$1" in',
      '        --match-head-commit) expected_head="$2"; shift 2 ;;',
      '        *) shift ;;',
      '      esac',
      '    done',
      '    "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" fetch origin "+refs/heads/*:refs/remotes/origin/*"',
      '    test "$expected_head" = "$("$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" rev-parse "origin/$head")"',
      '    "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" checkout -B main origin/main',
      '    "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" merge --squash "origin/$head"',
      '    "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" commit -m "merge snapshot PR"',
      '    "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" push origin HEAD:main',
      '    "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" push origin --delete "$head" >/dev/null 2>&1 || true',
      '    ;;',
      '  pr:view)',
      '    json=""',
      '    while [ "$#" -gt 0 ]; do',
      '      case "$1" in',
      '        --json) json="$2"; shift 2 ;;',
      '        *) shift ;;',
      '      esac',
      '    done',
      '    case "$json" in',
      '      state,mergeStateStatus,headRefOid,isDraft,statusCheckRollup)',
      '        head="$(cat "$SNAPSHOT_PR_HEAD_FILE")"',
      '        if "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" ls-remote --exit-code --heads origin "$head" >/dev/null 2>&1; then',
      '          state="OPEN"',
      '          head_sha="$("$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" ls-remote --heads origin "$head" | cut -f1)"',
      '        else',
      '          state="MERGED"',
      '          head_sha="$(cat "$SNAPSHOT_PR_HEAD_SHA_FILE")"',
      '        fi',
      '        printf \'{"state":"%s","mergeStateStatus":"CLEAN","headRefOid":"%s","isDraft":false,"statusCheckRollup":[]}\\n\' "$state" "$head_sha"',
      '        ;;',
      '      state)',
      '        head="$(cat "$SNAPSHOT_PR_HEAD_FILE")"',
      '        if "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" ls-remote --exit-code --heads origin "$head" >/dev/null 2>&1; then',
      '          echo "OPEN"',
      '        else',
      '          echo "MERGED"',
      '        fi',
      '        ;;',
      '      mergeCommit) "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" rev-parse HEAD ;;',
      '      *) exit 2 ;;',
      '    esac',
      '    ;;',
      '  pr:close)',
      '    head="$(cat "$SNAPSHOT_PR_HEAD_FILE")"',
      '    "$SNAPSHOT_REAL_GIT" -C "$SNAPSHOT_MERGE_REPO" push origin --delete "$head" >/dev/null 2>&1 || true',
      '    ;;',
      '  *) exit 2 ;;',
      'esac',
      '',
    ].join('\n'),
  )
  chmodSync(gh, 0o755)

  return {
    root,
    remote,
    source,
    competitor,
    wrapperDir,
    prHead,
    prHeadSha,
    realGit,
  }
}

function scriptEnv(fixture: ReturnType<typeof initFixture>) {
  return {
    GH_TOKEN: 'test-token',
    GITHUB_REPOSITORY: 'test/repo',
    GITHUB_RUN_ATTEMPT: '1',
    GITHUB_RUN_ID: '123',
    PATH: `${fixture.wrapperDir}:${process.env.PATH}`,
    SNAPSHOT_MERGE_REPO: fixture.competitor,
    SNAPSHOT_PR_HEAD_FILE: fixture.prHead,
    SNAPSHOT_PR_HEAD_SHA_FILE: fixture.prHeadSha,
    SNAPSHOT_REAL_GIT: fixture.realGit,
  }
}

describe('commit-update-snapshot', () => {
  it('preserves an unrelated snapshot committed before the PR merge', () => {
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

      const wrapper = join(fixture.wrapperDir, 'git')
      const marker = join(fixture.root, 'raced')
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
          'main',
          'snapshot BrowserOS server alpha 1.2.3',
          'updates/server/appcast-server.alpha.xml',
        ],
        {
          ...scriptEnv(fixture),
          SNAPSHOT_RACE_MARKER: marker,
          SNAPSHOT_RACE_REPO: fixture.competitor,
        },
      )

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(result.stdout).toContain('Snapshot PR merged')
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
      const result = run(
        fixture.source,
        [
          script,
          'main',
          'snapshot BrowserOS server alpha 1.2.3',
          'updates/server/appcast-server.alpha.xml',
        ],
        scriptEnv(fixture),
      )

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(result.stdout).toContain('Snapshots already current')
      expect(mustRun(fixture.remote, ['git', 'rev-parse', 'main'])).toBe(before)
    } finally {
      rmSync(fixture.root, { recursive: true, force: true })
    }
  })

  it('retries a transient pull request merge failure', () => {
    const fixture = initFixture()
    try {
      writeFileSync(
        join(fixture.source, 'updates/server/appcast-server.alpha.xml'),
        'server-new\n',
      )
      const result = run(
        fixture.source,
        [
          script,
          'main',
          'snapshot BrowserOS server alpha 1.2.3',
          'updates/server/appcast-server.alpha.xml',
        ],
        {
          ...scriptEnv(fixture),
          SNAPSHOT_MERGE_FAILURE_FILE: join(fixture.root, 'merge-failed-once'),
          SNAPSHOT_MERGE_POLL_SECONDS: '0',
        },
      )

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(result.stdout).toContain('Snapshot PR merged')
      expect(
        mustRun(fixture.root, [
          'git',
          `--git-dir=${fixture.remote}`,
          'show',
          'main:updates/server/appcast-server.alpha.xml',
        ]),
      ).toBe('server-new')
    } finally {
      rmSync(fixture.root, { recursive: true, force: true })
    }
  })

  it('rejects paths outside updates and missing snapshots', () => {
    const fixture = initFixture()
    try {
      const outside = run(
        fixture.source,
        [script, 'main', 'invalid snapshot', 'README.md'],
        scriptEnv(fixture),
      )
      const missing = run(
        fixture.source,
        [script, 'main', 'missing snapshot', 'updates/server/missing.xml'],
        scriptEnv(fixture),
      )

      expect(outside.code).not.toBe(0)
      expect(outside.stderr).toContain('must be under updates/')
      expect(missing.code).not.toBe(0)
      expect(missing.stderr).toContain('Snapshot does not exist')
    } finally {
      rmSync(fixture.root, { recursive: true, force: true })
    }
  })

  it('merges multiple feed files atomically through one pull request', () => {
    const fixture = initFixture()
    try {
      writeFileSync(
        join(fixture.source, 'updates/server/appcast-server.alpha.xml'),
        'server-new\n',
      )
      writeFileSync(
        join(fixture.source, 'updates/server/appcast-claw-server.alpha.xml'),
        'claw-new\n',
      )

      const result = run(
        fixture.source,
        [
          script,
          'main',
          'snapshot both feeds',
          'updates/server/appcast-server.alpha.xml',
          'updates/server/appcast-claw-server.alpha.xml',
        ],
        scriptEnv(fixture),
      )

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(
        mustRun(fixture.remote, ['git', 'rev-list', '--count', 'main']),
      ).toBe('2')
      expect(
        mustRun(fixture.remote, [
          'git',
          'show',
          'main:updates/server/appcast-server.alpha.xml',
        ]),
      ).toBe('server-new')
      expect(
        mustRun(fixture.remote, [
          'git',
          'show',
          'main:updates/server/appcast-claw-server.alpha.xml',
        ]),
      ).toBe('claw-new')
    } finally {
      rmSync(fixture.root, { recursive: true, force: true })
    }
  })
})
