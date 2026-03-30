/**
 * Debug script to test MCP server stability
 * Run with: bun apps/eval/scripts/debug-mcp.ts
 */

import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'

const SERVER_URL = 'http://127.0.0.1:9110'
const MCP_URL = `${SERVER_URL}/mcp`

interface TestResult {
  test: string
  success: boolean
  duration: number
  error?: string
}

const results: TestResult[] = []

async function checkHealth(): Promise<boolean> {
  try {
    const res = await fetch(`${SERVER_URL}/health`, {
      signal: AbortSignal.timeout(5000),
    })
    return res.ok
  } catch {
    return false
  }
}

async function checkBrowserReady(): Promise<boolean> {
  try {
    const res = await fetch(`${SERVER_URL}/health`, {
      signal: AbortSignal.timeout(5000),
    })
    if (!res.ok) return false
    const data = (await res.json()) as { cdpConnected?: boolean }
    return data.cdpConnected === true
  } catch {
    return false
  }
}

async function callMcpTool(
  name: string,
  args: Record<string, unknown> = {},
  timeoutMs: number = 30000,
): Promise<{
  success: boolean
  result?: unknown
  error?: string
  duration: number
}> {
  const start = Date.now()
  const client = new Client({ name: 'debug-script', version: '1.0.0' })
  const transport = new StreamableHTTPClientTransport(new URL(MCP_URL))

  try {
    await client.connect(transport)

    const toolPromise = client.callTool({ name, arguments: args })
    const timeoutPromise = new Promise<never>((_, reject) =>
      setTimeout(
        () => reject(new Error(`Timeout after ${timeoutMs}ms`)),
        timeoutMs,
      ),
    )

    const result = await Promise.race([toolPromise, timeoutPromise])
    const duration = Date.now() - start

    if ((result as any).isError) {
      const errorText =
        (result as any).content?.find((c: any) => c.type === 'text')?.text ||
        'Unknown error'
      return { success: false, error: errorText, duration }
    }

    return { success: true, result, duration }
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : String(error),
      duration: Date.now() - start,
    }
  } finally {
    try {
      await transport.close()
    } catch {}
  }
}

async function runTest(name: string, fn: () => Promise<void>): Promise<void> {
  const start = Date.now()
  try {
    await fn()
    results.push({ test: name, success: true, duration: Date.now() - start })
    console.log(`✅ ${name} (${Date.now() - start}ms)`)
  } catch (error) {
    const errorMsg = error instanceof Error ? error.message : String(error)
    results.push({
      test: name,
      success: false,
      duration: Date.now() - start,
      error: errorMsg,
    })
    console.log(`❌ ${name}: ${errorMsg} (${Date.now() - start}ms)`)
  }
}

async function main() {
  console.log('='.repeat(60))
  console.log('MCP Server Debug Script')
  console.log('='.repeat(60))
  console.log(`Server URL: ${SERVER_URL}`)
  console.log()

  // Phase 1: Basic connectivity
  console.log('\n--- Phase 1: Basic Connectivity ---\n')

  await runTest('Health check', async () => {
    const healthy = await checkHealth()
    if (!healthy) throw new Error('Server not healthy')
  })

  await runTest('Browser status', async () => {
    const connected = await checkBrowserReady()
    if (!connected) throw new Error('Browser not ready')
  })

  // Phase 2: List tools
  console.log('\n--- Phase 2: List Tools ---\n')

  let tools: string[] = []
  await runTest('List MCP tools', async () => {
    const client = new Client({ name: 'debug-script', version: '1.0.0' })
    const transport = new StreamableHTTPClientTransport(new URL(MCP_URL))
    try {
      await client.connect(transport)
      const result = await client.listTools()
      tools = result.tools.map((t) => t.name)
      console.log(`  Found ${tools.length} tools`)
    } finally {
      try {
        await transport.close()
      } catch {}
    }
  })

  // Phase 3: Create window and test tools
  console.log('\n--- Phase 3: Window & Screenshot Tests ---\n')

  let windowId: number | null = null
  let tabId: number | null = null

  await runTest('Create window', async () => {
    const res = await callMcpTool('browser_create_window', {
      url: 'https://example.com',
      focused: false,
    })
    if (!res.success) throw new Error(res.error)

    const structured = (res.result as any)?.structuredContent
    windowId = structured?.windowId
    tabId = structured?.tabId

    if (!windowId || !tabId) {
      // Try parsing from text
      const text =
        (res.result as any)?.content?.find((c: any) => c.type === 'text')
          ?.text || ''
      const windowMatch = text.match(/window\s+(\d+)/i)
      const tabMatch = text.match(/tab\s+(?:ID:\s*)?(\d+)/i)
      if (windowMatch) windowId = parseInt(windowMatch[1], 10)
      if (tabMatch) tabId = parseInt(tabMatch[1], 10)
    }

    if (!windowId || !tabId) throw new Error('Could not get windowId/tabId')
    console.log(`  Window: ${windowId}, Tab: ${tabId}`)
  })

  // Wait for page to load
  await new Promise((r) => setTimeout(r, 2000))

  // Phase 4: Screenshot stress test
  console.log('\n--- Phase 4: Screenshot Stress Test (10 screenshots) ---\n')

  let screenshotSuccesses = 0
  let screenshotFailures = 0

  for (let i = 1; i <= 10; i++) {
    const res = await callMcpTool(
      'browser_get_screenshot',
      {
        tabId,
        windowId,
        size: 'small',
      },
      65000,
    )

    if (res.success) {
      screenshotSuccesses++
      console.log(`  Screenshot ${i}: ✅ (${res.duration}ms)`)
    } else {
      screenshotFailures++
      console.log(`  Screenshot ${i}: ❌ ${res.error} (${res.duration}ms)`)
    }

    // Check browser status between screenshots
    const extConnected = await checkBrowserReady()
    if (!extConnected) {
      console.log(`  ⚠️  Browser became unavailable after screenshot ${i}!`)
    }

    // Small delay between screenshots
    await new Promise((r) => setTimeout(r, 500))
  }

  console.log(
    `\n  Screenshot results: ${screenshotSuccesses}/10 success, ${screenshotFailures}/10 failed`,
  )

  // Phase 5: Other tool tests
  console.log('\n--- Phase 5: Other Tool Tests ---\n')

  await runTest('Get active tab', async () => {
    const res = await callMcpTool('browser_get_active_tab', { windowId })
    if (!res.success) throw new Error(res.error)
  })

  await runTest('List tabs', async () => {
    const res = await callMcpTool('browser_list_tabs', { windowId })
    if (!res.success) throw new Error(res.error)
  })

  await runTest('Get interactive elements', async () => {
    const res = await callMcpTool('browser_get_interactive_elements', {
      tabId,
      windowId,
      simplified: true,
    })
    if (!res.success) throw new Error(res.error)
  })

  await runTest('Navigate', async () => {
    const res = await callMcpTool('browser_navigate', {
      url: 'https://google.com',
      tabId,
      windowId,
    })
    if (!res.success) throw new Error(res.error)
  })

  await new Promise((r) => setTimeout(r, 2000))

  await runTest('Get content snapshot', async () => {
    const res = await callMcpTool('browser_get_content', { tabId, windowId })
    if (!res.success) throw new Error(res.error)
  })

  // Phase 6: Cleanup
  console.log('\n--- Phase 6: Cleanup ---\n')

  if (windowId) {
    await runTest('Close window', async () => {
      const res = await callMcpTool('browser_close_window', { windowId })
      if (!res.success) throw new Error(res.error)
    })
  }

  // Final browser readiness check
  await runTest('Final browser status', async () => {
    const connected = await checkBrowserReady()
    if (!connected) throw new Error('Browser not ready')
  })

  // Summary
  console.log(`\n${'='.repeat(60)}`)
  console.log('SUMMARY')
  console.log('='.repeat(60))

  const passed = results.filter((r) => r.success).length
  const failed = results.filter((r) => !r.success).length
  const avgDuration =
    results.reduce((a, b) => a + b.duration, 0) / results.length

  console.log(`Total tests: ${results.length}`)
  console.log(`Passed: ${passed}`)
  console.log(`Failed: ${failed}`)
  console.log(`Avg duration: ${avgDuration.toFixed(0)}ms`)
  console.log(
    `Screenshot success rate: ${screenshotSuccesses}/10 (${screenshotSuccesses * 10}%)`,
  )

  if (failed > 0) {
    console.log('\nFailed tests:')
    for (const r of results.filter((r) => !r.success)) {
      console.log(`  - ${r.test}: ${r.error}`)
    }
  }

  console.log()
}

main().catch(console.error)
