// CLA check for pull requests, run by .github/workflows/cla-check.yml through
// actions/github-script. It resolves the PR from the API, compares the commit
// authors against browseros-ai/cla-signatures, records signatures posted as PR
// comments, reports a "cla" commit status, and keeps one sticky bot comment.

const SIGN_PHRASE = 'I have read the CLA Document and I hereby sign the CLA'
const STATUS_CONTEXT = 'cla'
const COMMENT_MARKER = '<!-- cla-check -->'
const LEGACY_COMMENT_TEXT = 'CLA Assistant Lite bot'
const BOT_LOGIN = 'github-actions[bot]'
const GITHUB_ACTIONS_BOT_ID = 41898282
const MAX_WRITE_ATTEMPTS = 3

const SIGNATURES = {
  owner: 'browseros-ai',
  repo: 'cla-signatures',
  path: 'cla-signatures.json',
  branch: 'main',
}

// Same entries and wildcard semantics as the previous CLA workflow: a pattern
// containing "*" is matched as an unanchored regex with "*" as ".*".
const ALLOWLIST = [
  'shadowfax92',
  'felarof99',
  'bot*',
  '*[bot]',
  'dependabot',
  'renovate',
  'github-actions',
  'snyk-bot',
  'imgbot',
  'greenkeeper',
  'semantic-release-bot',
  'allcontributors',
]

const COMMITS_QUERY = `
  query ($owner: String!, $repo: String!, $number: Int!, $cursor: String) {
    repository(owner: $owner, name: $repo) {
      pullRequest(number: $number) {
        commits(first: 100, after: $cursor) {
          pageInfo {
            hasNextPage
            endCursor
          }
          nodes {
            commit {
              author {
                name
                user {
                  databaseId
                  login
                }
              }
              committer {
                name
                user {
                  databaseId
                  login
                }
              }
            }
          }
        }
      }
    }
  }
`

function notSignedComment(docUrl) {
  return `Thank you for your contribution! Before we can merge this PR, we need you to sign our [Contributor License Agreement](${docUrl}).

**To sign the CLA**, please add a comment to this PR with the following text:

\`\`\`
${SIGN_PHRASE}
\`\`\`

You only need to sign once. After signing, this check will pass automatically.

---
<details>
<summary>Troubleshooting</summary>

- **Already signed but still failing?** Comment \`recheck\` to trigger a re-verification.
- **Signed with a different email?** Make sure your commit email matches your GitHub account email, or add your commit email to your GitHub account.

</details>
`
}

const ALL_SIGNED_COMMENT = `All contributors have signed the CLA. Thank you!
`

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

function isAllowlisted(name) {
  return ALLOWLIST.some((pattern) => {
    if (pattern.includes('*')) {
      const regex = escapeRegExp(pattern).split('\\*').join('.*')
      return new RegExp(regex).test(name)
    }
    return pattern === name
  })
}

function isSignComment(body) {
  return (body || '').trim().toLowerCase() === SIGN_PHRASE.toLowerCase()
}

// Git author names are free text; keep them inert inside the comment.
function codeSpan(text) {
  return `\`${String(text).replace(/[`\r\n]/g, '')}\``
}

module.exports = async ({ github, context, core, getOctokit }) => {
  const token = process.env.SIGNATURES_TOKEN
  if (!token) {
    core.setFailed('CLA_SIGNATURES_TOKEN is not configured')
    return
  }
  const signaturesClient = getOctokit(token)
  const { owner, repo } = context.repo
  const docUrl = `https://github.com/${owner}/${repo}/blob/main/CLA.md`

  async function readSignatures() {
    const { data } = await signaturesClient.rest.repos.getContent({
      owner: SIGNATURES.owner,
      repo: SIGNATURES.repo,
      path: SIGNATURES.path,
      ref: SIGNATURES.branch,
    })
    const file = JSON.parse(Buffer.from(data.content, 'base64').toString('utf8'))
    if (!Array.isArray(file.signedContributors)) {
      throw new Error(`${SIGNATURES.path} has no signedContributors array`)
    }
    return { file, sha: data.sha }
  }

  // Appends records with an optimistic lock on the file sha, re-reading and
  // retrying when another run wrote in between. Returns the updated list.
  async function recordSignatures(records, prNumber) {
    for (let attempt = 1; ; attempt++) {
      const { file, sha } = await readSignatures()
      const fresh = records.filter(
        (record) => !file.signedContributors.some((s) => s.id === record.id),
      )
      if (fresh.length === 0) return file.signedContributors
      file.signedContributors.push(...fresh)
      const names = fresh.map((record) => `@${record.name}`).join(', ')
      try {
        await signaturesClient.rest.repos.createOrUpdateFileContents({
          owner: SIGNATURES.owner,
          repo: SIGNATURES.repo,
          path: SIGNATURES.path,
          branch: SIGNATURES.branch,
          sha,
          message: `${names} has signed the CLA in ${owner}/${repo}#${prNumber}`,
          content: Buffer.from(JSON.stringify(file, null, 2)).toString('base64'),
        })
        core.info(`Recorded CLA signature for ${names} on #${prNumber}`)
        return file.signedContributors
      } catch (error) {
        if (error.status === 409 && attempt < MAX_WRITE_ATTEMPTS) {
          core.info(`Signature write conflicted, retrying (${attempt})`)
          continue
        }
        throw error
      }
    }
  }

  async function listCommitters(prNumber) {
    const committers = []
    let cursor = null
    do {
      const result = await github.graphql(COMMITS_QUERY, {
        owner,
        repo,
        number: prNumber,
        cursor,
      })
      const commits = result.repository.pullRequest.commits
      for (const { commit } of commits.nodes) {
        const user = commit.author?.user || commit.committer?.user
        const committer = user
          ? { name: user.login, id: user.databaseId }
          : {
              name: commit.author?.name || commit.committer?.name || 'unknown',
              id: null,
            }
        if (!committers.some((c) => c.name === committer.name)) {
          committers.push(committer)
        }
      }
      cursor = commits.pageInfo.hasNextPage ? commits.pageInfo.endCursor : null
    } while (cursor)
    return committers.filter((c) => c.id !== GITHUB_ACTIONS_BOT_ID)
  }

  function classify(committers, signatures) {
    const relevant = committers.filter((c) => !isAllowlisted(c.name))
    const signed = relevant.filter(
      (c) => c.id && signatures.some((s) => s.id === c.id),
    )
    const notSigned = relevant.filter((c) => !signed.includes(c))
    const unknown = notSigned.filter((c) => !c.id)
    return { signed, notSigned, unknown }
  }

  function buildComment(result) {
    const { signed, notSigned, unknown } = result
    if (notSigned.length === 0) return `${ALL_SIGNED_COMMENT}${COMMENT_MARKER}`

    let body = notSignedComment(docUrl)
    const total = signed.length + notSigned.length
    if (total > 1) {
      body += `\n**${signed.length}** out of **${total}** committers have signed the CLA.\n\n`
      for (const c of signed) body += `- :white_check_mark: ${codeSpan(c.name)}\n`
      for (const c of notSigned) {
        body += `- :x: ${c.id ? `@${c.name}` : codeSpan(c.name)}\n`
      }
    }
    if (unknown.length > 0) {
      const names = unknown.map((c) => codeSpan(c.name)).join(', ')
      const seem = unknown.length > 1 ? 'seem' : 'seems'
      body += `\n${names} ${seem} not to be a GitHub user. You need a GitHub account to be able to sign the CLA. If you already have a GitHub account, please [add the email address used for this commit to your account](https://docs.github.com/en/pull-requests/committing-changes-to-your-project/troubleshooting-commits/why-are-my-commits-linked-to-the-wrong-user#commits-are-not-linked-to-any-user).\n`
    }
    return `${body}\n${COMMENT_MARKER}`
  }

  function findStickyComment(comments) {
    const own = comments.filter((c) => c.user?.login === BOT_LOGIN)
    return (
      own.find((c) => c.body?.includes(COMMENT_MARKER)) ||
      own.find((c) => c.body?.includes(LEGACY_COMMENT_TEXT))
    )
  }

  async function updateStickyComment(prNumber, comments, result) {
    const existing = findStickyComment(comments)
    const body = buildComment(result)
    if (existing) {
      if (existing.body !== body) {
        await github.rest.issues.updateComment({
          owner,
          repo,
          comment_id: existing.id,
          body,
        })
      }
      return
    }
    // Like before, a PR that is fully signed from the start gets no comment.
    if (result.notSigned.length === 0) return
    await github.rest.issues.createComment({
      owner,
      repo,
      issue_number: prNumber,
      body,
    })
  }

  async function hasClaStatus(sha, state) {
    const { data } = await github.rest.repos.getCombinedStatusForRef({
      owner,
      repo,
      ref: sha,
      per_page: 100,
    })
    return data.statuses.some(
      (s) => s.context === STATUS_CONTEXT && (!state || s.state === state),
    )
  }

  // Signatures are only ever appended, so a success on a commit is final. A
  // run that read the signatures before another run recorded one must not
  // turn that success back into a failure. Returns false when it skips.
  async function setStatus(sha, result) {
    if (result.notSigned.length > 0 && (await hasClaStatus(sha, 'success'))) {
      core.info(`${sha} already has a successful ${STATUS_CONTEXT} status`)
      return false
    }
    let description = 'All contributors have signed the CLA'
    if (result.notSigned.length > 0) {
      description = `CLA not signed by: ${result.notSigned.map((c) => c.name).join(', ')}`
      if (description.length > 140) description = `${description.slice(0, 137)}...`
    }
    await github.rest.repos.createCommitStatus({
      owner,
      repo,
      sha,
      state: result.notSigned.length > 0 ? 'failure' : 'success',
      context: STATUS_CONTEXT,
      description,
      target_url: docUrl,
    })
    return true
  }

  async function checkPullRequest(pr) {
    const committers = await listCommitters(pr.number)
    const comments = await github.paginate(github.rest.issues.listComments, {
      owner,
      repo,
      issue_number: pr.number,
      per_page: 100,
    })

    // Read per PR so signatures recorded by other runs are picked up.
    let result = classify(
      committers,
      (await readSignatures()).file.signedContributors,
    )

    // Any listed, unsigned author who has commented the sign phrase on this PR
    // gets recorded, using their earliest such comment.
    const records = []
    for (const committer of result.notSigned.filter((c) => c.id)) {
      const comment = comments.find(
        (c) => c.user?.id === committer.id && isSignComment(c.body),
      )
      if (!comment) continue
      records.push({
        name: comment.user.login,
        id: comment.user.id,
        comment_id: comment.id,
        created_at: comment.created_at,
        repoId: pr.base.repo.id,
        pullRequestNo: pr.number,
      })
    }
    if (records.length > 0) {
      result = classify(committers, await recordSignatures(records, pr.number))
    }

    // A stale result must not rewrite the comment either.
    if (!(await setStatus(pr.head.sha, result))) return
    await updateStickyComment(pr.number, comments, result)
    core.info(
      `#${pr.number} ${pr.head.sha}: ${
        result.notSigned.length > 0
          ? `unsigned: ${result.notSigned.map((c) => c.name).join(', ')}`
          : 'all signed'
      }`,
    )
  }

  async function getPullRequest(number) {
    const { data } = await github.rest.pulls.get({
      owner,
      repo,
      pull_number: number,
    })
    return data
  }

  async function listOpenPullRequests() {
    return github.paginate(github.rest.pulls.list, {
      owner,
      repo,
      state: 'open',
      per_page: 100,
    })
  }

  // The PR is derived from the API, never from the triggering run's outputs.
  async function pullRequestsForWorkflowRun() {
    const run = context.payload.workflow_run
    const headOwner = run.head_repository?.owner?.login
    if (!headOwner || !run.head_branch || !run.head_sha) {
      core.info('Workflow run has no head repository or branch, nothing to check')
      return []
    }
    const pulls = await github.paginate(github.rest.pulls.list, {
      owner,
      repo,
      state: 'open',
      head: `${headOwner}:${run.head_branch}`,
      per_page: 100,
    })
    return pulls.filter((pr) => pr.head.sha === run.head_sha)
  }

  let pulls = []
  switch (context.eventName) {
    case 'workflow_run':
      pulls = await pullRequestsForWorkflowRun()
      break
    case 'issue_comment':
      if (!context.payload.issue?.pull_request) return
      pulls = [await getPullRequest(context.payload.issue.number)]
      break
    case 'workflow_dispatch': {
      const input = String(context.payload.inputs?.pr || '').trim()
      if (input) {
        const number = Number(input)
        if (!Number.isInteger(number) || number <= 0) {
          core.setFailed(`Invalid PR number: ${input}`)
          return
        }
        pulls = [await getPullRequest(number)]
      } else {
        pulls = await listOpenPullRequests()
      }
      break
    }
    case 'schedule': {
      const open = await listOpenPullRequests()
      for (const pr of open) {
        if (!(await hasClaStatus(pr.head.sha))) pulls.push(pr)
      }
      break
    }
    default:
      core.info(`Unsupported event ${context.eventName}`)
      return
  }

  if (pulls.length === 0) {
    core.info('No pull requests to check')
    return
  }

  let failures = 0
  for (const pr of pulls) {
    try {
      await checkPullRequest(pr)
    } catch (error) {
      failures++
      core.error(`#${pr.number}: ${error.message}`)
    }
  }
  if (failures > 0) {
    core.setFailed(`CLA check failed for ${failures} pull request(s)`)
  }
}
