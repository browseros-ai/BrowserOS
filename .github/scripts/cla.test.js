// Tests for cla.js against mocked GitHub clients. Run: node --test .github/scripts/cla.test.js

const assert = require('node:assert/strict')
const { test } = require('node:test')
const run = require('./cla.js')

const SIGN_PHRASE = 'I have read the CLA Document and I hereby sign the CLA'
const MARKER = '<!-- cla-check -->'
const REPO_ID = 985839104

function pr(number, sha, login = 'forker') {
  return {
    number,
    head: { sha, ref: 'feature', repo: { owner: { login } } },
    base: { repo: { id: REPO_ID } },
  }
}

function author(login, id) {
  return {
    author: { name: login, user: { login, databaseId: id } },
    committer: { name: 'GitHub', user: null },
  }
}

function unlinkedAuthor(name) {
  return {
    author: { name, user: null },
    committer: { name, user: null },
  }
}

function comment(id, login, userId, body, createdAt = '2026-11-09T00:00:00Z') {
  return { id, user: { login, id: userId }, body, created_at: createdAt }
}

function botComment(id, body) {
  return { id, user: { login: 'github-actions[bot]', id: 41898282 }, body }
}

// Builds mocked clients around a mutable state object. `statuses` maps a sha
// to existing commit statuses; `conflicts` makes that many signature writes
// fail with 409; `onSignaturesRead` runs after each signatures read.
function setup({
  eventName,
  payload = {},
  prs,
  commits,
  comments = [],
  signatures = [],
  statuses = {},
  conflicts = 0,
  onSignaturesRead,
}) {
  const state = {
    file: { signedContributors: signatures },
    fileSha: 'sha-0',
    conflicts,
    reads: 0,
    writes: [],
    statuses: Object.fromEntries(
      Object.entries(statuses).map(([sha, list]) => [sha, [...list]]),
    ),
    statusWrites: [],
    comments: [...comments],
    created: 0,
    updated: 0,
    failed: null,
    errors: [],
  }

  const commitPage = (cursor) => {
    const nodes = cursor ? commits.slice(100) : commits.slice(0, 100)
    return {
      repository: {
        pullRequest: {
          commits: {
            pageInfo: {
              hasNextPage: !cursor && commits.length > 100,
              endCursor: 'cursor-1',
            },
            nodes: nodes.map((commit) => ({ commit })),
          },
        },
      },
    }
  }

  const github = {
    graphql: async (_query, variables) => commitPage(variables.cursor),
    paginate: async (method, params) => (await method(params)).data,
    rest: {
      pulls: {
        get: async ({ pull_number }) => ({
          data: prs.find((p) => p.number === pull_number),
        }),
        list: async ({ head }) => ({
          data: prs.filter(
            (p) => !head || head === `${p.head.repo.owner.login}:${p.head.ref}`,
          ),
        }),
      },
      issues: {
        listComments: async () => ({ data: state.comments }),
        createComment: async ({ body }) => {
          state.created++
          state.comments.push(botComment(1000 + state.created, body))
        },
        updateComment: async ({ comment_id, body }) => {
          state.updated++
          state.comments.find((c) => c.id === comment_id).body = body
        },
      },
      repos: {
        createCommitStatus: async (status) => {
          state.statusWrites.push(status)
          const list = state.statuses[status.sha] || []
          state.statuses[status.sha] = [
            ...list.filter((s) => s.context !== status.context),
            status,
          ]
        },
        getCombinedStatusForRef: async ({ ref }) => ({
          data: { statuses: state.statuses[ref] || [] },
        }),
      },
    },
  }

  const signaturesClient = {
    rest: {
      repos: {
        getContent: async () => {
          const data = {
            sha: state.fileSha,
            content: Buffer.from(JSON.stringify(state.file, null, 2)).toString(
              'base64',
            ),
          }
          state.reads++
          if (onSignaturesRead) onSignaturesRead(state)
          return { data }
        },
        createOrUpdateFileContents: async (params) => {
          if (state.conflicts > 0) {
            state.conflicts--
            state.fileSha = `sha-${state.writes.length}-conflict-${state.conflicts}`
            const error = new Error('conflict')
            error.status = 409
            throw error
          }
          assert.equal(params.sha, state.fileSha)
          const raw = Buffer.from(params.content, 'base64').toString('utf8')
          state.file = JSON.parse(raw)
          state.fileSha = `sha-${state.writes.length + 1}`
          state.writes.push({ ...params, raw })
        },
      },
    },
  }

  const core = {
    info: () => {},
    error: (message) => state.errors.push(message),
    setFailed: (message) => {
      state.failed = message
    },
  }

  const args = {
    github,
    context: {
      eventName,
      payload,
      repo: { owner: 'browseros-ai', repo: 'BrowserOS' },
    },
    core,
    getOctokit: () => signaturesClient,
  }
  return { state, args }
}

process.env.SIGNATURES_TOKEN = 'test-token'

test('unsigned, allowlisted, and unknown authors on a workflow run', async () => {
  const { state, args } = setup({
    eventName: 'workflow_run',
    payload: {
      workflow_run: {
        head_sha: 'abc',
        head_branch: 'feature',
        head_repository: { owner: { login: 'forker' } },
      },
    },
    prs: [pr(10, 'abc'), pr(11, 'stale')],
    commits: [
      author('forker', 1),
      author('renovate[bot]', 2),
      author('shadowfax92', 3),
      unlinkedAuthor('Nobody'),
    ],
    signatures: [{ name: 'someone', id: 99 }],
  })
  await run(args)

  assert.equal(state.statusWrites.length, 1)
  const [status] = state.statusWrites
  assert.equal(status.sha, 'abc')
  assert.equal(status.context, 'cla')
  assert.equal(status.state, 'failure')
  assert.equal(status.description, 'CLA not signed by: forker, Nobody')
  assert.equal(
    status.target_url,
    'https://github.com/browseros-ai/BrowserOS/blob/main/CLA.md',
  )

  assert.equal(state.created, 1)
  const body = state.comments[0].body
  assert.ok(body.startsWith('Thank you for your contribution!'))
  assert.ok(body.includes(`\`\`\`\n${SIGN_PHRASE}\n\`\`\``))
  assert.ok(body.includes('**0** out of **2** committers have signed the CLA.'))
  assert.ok(body.includes('- :x: @forker'))
  assert.ok(body.includes('`Nobody` seems not to be a GitHub user.'))
  assert.ok(body.endsWith(MARKER))
  assert.equal(state.failed, null)
})

test('allowlist wildcards match anywhere in the name', async () => {
  const { state, args } = setup({
    eventName: 'issue_comment',
    payload: { issue: { number: 10, pull_request: {} } },
    prs: [pr(10, 'abc')],
    commits: [author('dependabot[bot]', 5), author('robotics-dev', 6)],
  })
  await run(args)

  assert.equal(state.statusWrites[0].state, 'success')
  assert.equal(state.created, 0)
})

test('signing comment records the earliest signature after a 409 retry', async () => {
  const { state, args } = setup({
    eventName: 'issue_comment',
    payload: {
      issue: { number: 10, pull_request: {} },
      comment: { body: SIGN_PHRASE },
    },
    prs: [pr(10, 'abc')],
    commits: Array.from({ length: 150 }, () => author('forker', 1)),
    comments: [
      botComment(5, '**CLA Assistant Lite bot:** Thank you'),
      comment(6, 'bystander', 7, SIGN_PHRASE),
      comment(7, 'forker', 1, `${SIGN_PHRASE}\n`, '2026-11-09T00:00:00Z'),
      comment(8, 'forker', 1, SIGN_PHRASE, '2026-11-09T01:00:00Z'),
    ],
    signatures: [{ name: 'someone', id: 99 }],
    conflicts: 1,
  })
  await run(args)

  assert.equal(state.writes.length, 1)
  const [write] = state.writes
  assert.equal(write.owner, 'browseros-ai')
  assert.equal(write.repo, 'cla-signatures')
  assert.equal(write.path, 'cla-signatures.json')
  assert.equal(write.branch, 'main')
  assert.equal(
    write.message,
    '@forker has signed the CLA in browseros-ai/BrowserOS#10',
  )
  assert.ok(!write.raw.endsWith('\n'))
  assert.deepEqual(state.file.signedContributors, [
    { name: 'someone', id: 99 },
    {
      name: 'forker',
      id: 1,
      comment_id: 7,
      created_at: '2026-11-09T00:00:00Z',
      repoId: REPO_ID,
      pullRequestNo: 10,
    },
  ])

  assert.equal(state.statusWrites[0].state, 'success')
  assert.equal(state.updated, 1)
  assert.equal(
    state.comments[0].body,
    `All contributors have signed the CLA. Thank you!\n${MARKER}`,
  )
  assert.equal(state.failed, null)
})

test('signature write fails the run once retries run out', async () => {
  const { state, args } = setup({
    eventName: 'issue_comment',
    payload: { issue: { number: 10, pull_request: {} } },
    prs: [pr(10, 'abc')],
    commits: [author('forker', 1)],
    comments: [comment(7, 'forker', 1, SIGN_PHRASE)],
    conflicts: 5,
  })
  await run(args)

  assert.equal(state.writes.length, 0)
  assert.equal(state.statusWrites.length, 0)
  assert.match(state.failed, /failed for 1 pull request/)
})

test('a fully signed PR gets a success status and no comment', async () => {
  const { state, args } = setup({
    eventName: 'issue_comment',
    payload: { issue: { number: 10, pull_request: {} } },
    prs: [pr(10, 'abc')],
    commits: [author('forker', 1)],
    signatures: [{ name: 'forker', id: 1 }],
  })
  await run(args)

  assert.equal(state.statusWrites[0].state, 'success')
  assert.equal(state.created, 0)
  assert.equal(state.updated, 0)
})

test('sticky comment is created once, then updated in place', async () => {
  const first = setup({
    eventName: 'issue_comment',
    payload: { issue: { number: 10, pull_request: {} } },
    prs: [pr(10, 'abc')],
    commits: [author('forker', 1)],
  })
  await run(first.args)
  assert.equal(first.state.created, 1)

  const second = setup({
    eventName: 'issue_comment',
    payload: { issue: { number: 10, pull_request: {} } },
    prs: [pr(10, 'abc')],
    commits: [author('forker', 1)],
    comments: [...first.state.comments, comment(20, 'forker', 1, SIGN_PHRASE)],
  })
  await run(second.args)
  assert.equal(second.state.created, 0)
  assert.equal(second.state.updated, 1)
  const sticky = second.state.comments.filter((c) => c.body.includes(MARKER))
  assert.equal(sticky.length, 1)
  assert.ok(sticky[0].body.startsWith('All contributors have signed the CLA.'))
})

test('scheduled sweep only checks heads without a cla status', async () => {
  const { state, args } = setup({
    eventName: 'schedule',
    prs: [pr(1, 'checked'), pr(2, 'unchecked')],
    commits: [author('forker', 1)],
    statuses: { checked: [{ context: 'cla', state: 'failure' }] },
  })
  await run(args)

  assert.deepEqual(
    state.statusWrites.map((s) => s.sha),
    ['unchecked'],
  )
})

test('manual dispatch without input checks every open PR', async () => {
  const { state, args } = setup({
    eventName: 'workflow_dispatch',
    payload: { inputs: { pr: '' } },
    prs: [pr(1, 'checked'), pr(2, 'unchecked')],
    commits: [author('forker', 1)],
    statuses: { checked: [{ context: 'cla', state: 'failure' }] },
  })
  await run(args)

  assert.deepEqual(
    state.statusWrites.map((s) => s.sha),
    ['checked', 'unchecked'],
  )
})

test('a failure never replaces an existing cla success on the same head', async () => {
  const { state, args } = setup({
    eventName: 'workflow_dispatch',
    payload: { inputs: {} },
    prs: [pr(1, 'signed-head'), pr(2, 'failing-head')],
    commits: [author('forker', 1)],
    statuses: {
      'signed-head': [{ context: 'cla', state: 'success' }],
      'failing-head': [{ context: 'cla', state: 'failure' }],
    },
  })
  await run(args)

  assert.deepEqual(
    state.statusWrites.map((s) => [s.sha, s.state]),
    [['failing-head', 'failure']],
  )
  assert.equal(state.statuses['signed-head'][0].state, 'success')
  // The skipped run leaves the sticky comment alone as well.
  const solo = setup({
    eventName: 'workflow_dispatch',
    payload: { inputs: { pr: '1' } },
    prs: [pr(1, 'signed-head')],
    commits: [author('forker', 1)],
    comments: [botComment(5, `All contributors have signed the CLA. Thank you!\n${MARKER}`)],
    statuses: { 'signed-head': [{ context: 'cla', state: 'success' }] },
  })
  await run(solo.args)
  assert.equal(solo.state.statusWrites.length, 0)
  assert.equal(solo.state.updated, 0)
  assert.equal(solo.state.created, 0)
})

test('a failure can be upgraded to success', async () => {
  const { state, args } = setup({
    eventName: 'issue_comment',
    payload: { issue: { number: 10, pull_request: {} } },
    prs: [pr(10, 'abc')],
    commits: [author('forker', 1)],
    signatures: [{ name: 'forker', id: 1 }],
    statuses: { abc: [{ context: 'cla', state: 'failure' }] },
  })
  await run(args)

  assert.equal(state.statuses.abc[0].state, 'success')
})

test('signatures are re-read for each PR in a sweep', async () => {
  const { state, args } = setup({
    eventName: 'workflow_dispatch',
    payload: { inputs: {} },
    prs: [pr(1, 'first'), pr(2, 'second')],
    commits: [author('forker', 1)],
    // Another run records the signature right after the first PR's read.
    onSignaturesRead: (s) => {
      if (s.reads === 1) {
        s.file = { signedContributors: [{ name: 'forker', id: 1 }] }
      }
    },
  })
  await run(args)

  assert.equal(state.reads, 2)
  assert.deepEqual(
    state.statusWrites.map((s) => [s.sha, s.state]),
    [
      ['first', 'failure'],
      ['second', 'success'],
    ],
  )
})
