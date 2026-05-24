import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'

async function main() {
  const url = 'http://127.0.0.1:9203/mcp'
  const transport = new StreamableHTTPClientTransport(new URL(url))
  const client = new Client({ name: 'e2e-test', version: '0.1.0' })
  await client.connect(transport)

  console.log('=== Test 1: github_repo_info ===')
  const repo = await client.callTool({
    name: 'github_repo_info',
    arguments: { owner: 'gHashTag', repo: 'trinity' },
  })
  const repoText = repo.content?.[0]?.text || ''
  console.log(repoText.slice(0, 400))
  console.log(
    '✅ PASS:',
    repoText.includes('gHashTag/trinity') ? 'found repo name' : 'FAIL',
  )

  console.log('\n=== Test 2: github_read_file ===')
  const file = await client.callTool({
    name: 'github_read_file',
    arguments: { owner: 'gHashTag', repo: 'trinity', path: 'README.md' },
  })
  const fileText = file.content?.[0]?.text || ''
  console.log(fileText.slice(0, 400))
  console.log(
    '✅ PASS:',
    fileText.includes('Trinity') ? 'found Trinity in README' : 'FAIL',
  )

  console.log('\n=== Test 3: github_list_issues ===')
  const issues = await client.callTool({
    name: 'github_list_issues',
    arguments: {
      owner: 'gHashTag',
      repo: 'trinity',
      state: 'open',
      per_page: 3,
    },
  })
  const issuesText = issues.content?.[0]?.text || ''
  console.log(issuesText.slice(0, 400))
  console.log(
    '✅ PASS:',
    issuesText.includes('"number"') || issuesText.includes('count')
      ? 'got issues list'
      : 'FAIL',
  )

  console.log('\n=== Test 4: github_search_code ===')
  const search = await client.callTool({
    name: 'github_search_code',
    arguments: { query: 'phi repo:gHashTag/trinity', per_page: 5 },
  })
  const searchText = search.content?.[0]?.text || ''
  console.log(searchText.slice(0, 400))
  console.log(
    '✅ PASS:',
    searchText.includes('"total_count"') ? 'got search results' : 'FAIL',
  )

  console.log('\n=== Test 5: github_create_issue (dry-run) ===')
  const issue = await client.callTool({
    name: 'github_create_issue',
    arguments: {
      owner: 'gHashTag',
      repo: 'trinity',
      title: 'Test issue from bridge E2E',
      dry_run: true,
    },
  })
  const issueText = issue.content?.[0]?.text || ''
  console.log(issueText)
  console.log(
    '✅ PASS:',
    issueText.includes('DRY RUN') ? 'dry-run works' : 'FAIL',
  )

  console.log('\n=== Test 6: github_list_branches ===')
  const branches = await client.callTool({
    name: 'github_list_branches',
    arguments: { owner: 'gHashTag', repo: 'trinity', per_page: 5 },
  })
  const branchesText = branches.content?.[0]?.text || ''
  console.log(branchesText.slice(0, 300))
  console.log(
    '✅ PASS:',
    branchesText.includes('dev') || branchesText.includes('main')
      ? 'found branches'
      : 'FAIL',
  )

  console.log('\n=== Test 7: github_list_commits ===')
  const commits = await client.callTool({
    name: 'github_list_commits',
    arguments: { owner: 'gHashTag', repo: 'trinity', per_page: 3 },
  })
  const commitsText = commits.content?.[0]?.text || ''
  console.log(commitsText.slice(0, 300))
  console.log(
    '✅ PASS:',
    commitsText.includes('"sha"') || commitsText.includes('"message"')
      ? 'got commits'
      : 'FAIL',
  )

  await client.close()
  await transport.close()
  console.log('\n🎉 ALL TESTS COMPLETE')
}

main().catch(console.error)
