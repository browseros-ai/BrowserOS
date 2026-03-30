/**
 * Long-running stress test to simulate eval behavior
 * Run with: bun apps/eval/scripts/debug-long-run.ts
 */

import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'

const SERVER_URL = 'http://127.0.0.1:9110'
const MCP_URL = `${SERVER_URL}/mcp`

// Simulate 60 turns like the failing task had
const NUM_TURNS = 60
const SCREENSHOT_EVERY_N_TURNS = 1

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
  timeoutMs: number = 65000,
): Promise<{ success: boolean; error?: string; duration: number }> {
  const start = Date.now()
  const client = new Client({ name: 'long-run-test', version: '1.0.0' })
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

    const res = result as Record<string, unknown>
    if (res.isError) {
      const content = res.content as
        | Array<{ type: string; text?: string }>
        | undefined
      const errorText =
        content?.find((c) => c.type === 'text')?.text || 'Unknown error'
      return { success: false, error: errorText, duration }
    }

    return { success: true, duration }
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

async function main() {
  console.log('='.repeat(60))
  console.log('Long-Running Stress Test (simulating eval)')
  console.log('='.repeat(60))
  console.log(
    `Simulating ${NUM_TURNS} turns with screenshots every ${SCREENSHOT_EVERY_N_TURNS} turn(s)`,
  )
  console.log()

  // Create window
  console.log('Creating window...')

  let windowId = 0
  let tabId = 0

  const client = new Client({ name: 'long-run-test', version: '1.0.0' })
  const transport = new StreamableHTTPClientTransport(new URL(MCP_URL))

  try {
    await client.connect(transport)
    const result = await client.callTool({
      name: 'browser_create_window',
      arguments: { url: 'https://example.com', focused: false },
    })

    // Try structured content first
    const createRes = result as Record<string, unknown>
    const structured = createRes.structuredContent as
      | Record<string, number>
      | undefined
    windowId = structured?.windowId ?? 0
    tabId = structured?.tabId ?? 0

    // Fall back to parsing text
    if (!windowId || !tabId) {
      const content = createRes.content as
        | Array<{ type: string; text?: string }>
        | undefined
      const text = content?.find((c) => c.type === 'text')?.text || ''
      const windowMatch = text.match(/window\s+(\d+)/i)
      const tabMatch =
        text.match(/Tab ID:\s*(\d+)/i) || text.match(/tab\s+(\d+)/i)
      if (windowMatch) windowId = parseInt(windowMatch[1], 10)
      if (tabMatch) tabId = parseInt(tabMatch[1], 10)
    }
  } finally {
    try {
      await transport.close()
    } catch {}
  }

  if (!windowId || !tabId) {
    console.log('❌ Could not determine window/tab IDs')
    console.log('Trying to get from list tabs...')

    // Try listing tabs
    const client2 = new Client({ name: 'long-run-test', version: '1.0.0' })
    const transport2 = new StreamableHTTPClientTransport(new URL(MCP_URL))
    try {
      await client2.connect(transport2)
      const tabs = await client2.callTool({
        name: 'browser_list_tabs',
        arguments: {},
      })
      console.log('Tabs response:', JSON.stringify(tabs, null, 2))
    } finally {
      try {
        await transport2.close()
      } catch {}
    }
    return
  }

  console.log(`Window: ${windowId}, Tab: ${tabId}`)
  console.log()

  await new Promise((r) => setTimeout(r, 2000))

  // Stats
  let screenshotSuccess = 0
  let screenshotFail = 0
  let toolSuccess = 0
  let toolFail = 0
  let browserDisconnects = 0

  const startTime = Date.now()

  // Simulate turns
  for (let turn = 1; turn <= NUM_TURNS; turn++) {
    const _turnStart = Date.now()

    // Random tool calls to simulate agent behavior
    const tools = [
      {
        name: 'browser_get_interactive_elements',
        args: { tabId, windowId, simplified: true },
      },
      { name: 'browser_list_tabs', args: { windowId } },
      { name: 'browser_get_active_tab', args: { windowId } },
    ]

    // Pick a random tool
    const tool = tools[Math.floor(Math.random() * tools.length)]
    const toolRes = await callMcpTool(tool.name, tool.args, 30000)

    if (toolRes.success) {
      toolSuccess++
    } else {
      toolFail++
      console.log(`  Turn ${turn}: ❌ ${tool.name} failed: ${toolRes.error}`)
    }

    // Screenshot every N turns
    if (turn % SCREENSHOT_EVERY_N_TURNS === 0) {
      const ssRes = await callMcpTool(
        'browser_get_screenshot',
        { tabId, windowId, size: 'small' },
        65000,
      )

      if (ssRes.success) {
        screenshotSuccess++
      } else {
        screenshotFail++
        console.log(`  Turn ${turn}: ❌ Screenshot failed: ${ssRes.error}`)
      }
    }

    // Check browser status
    const browserReady = await checkBrowserReady()
    if (!browserReady) {
      browserDisconnects++
      console.log(`  Turn ${turn}: ⚠️ Browser became unavailable!`)
    }

    // Progress
    if (turn % 10 === 0) {
      const elapsed = ((Date.now() - startTime) / 1000).toFixed(1)
      console.log(
        `Turn ${turn}/${NUM_TURNS} - Screenshots: ${screenshotSuccess}/${turn}, Tools: ${toolSuccess}/${turn}, Disconnects: ${browserDisconnects}, Elapsed: ${elapsed}s`,
      )
    }

    // Small delay between turns
    await new Promise((r) => setTimeout(r, 200))
  }

  // Cleanup
  console.log('\nClosing window...')
  await callMcpTool('browser_close_window', { windowId })

  // Summary
  const totalTime = ((Date.now() - startTime) / 1000).toFixed(1)

  console.log(`\n${'='.repeat(60)}`)
  console.log('SUMMARY')
  console.log('='.repeat(60))
  console.log(`Total time: ${totalTime}s`)
  console.log(
    `Screenshots: ${screenshotSuccess}/${NUM_TURNS} (${((screenshotSuccess / NUM_TURNS) * 100).toFixed(1)}%)`,
  )
  console.log(
    `Tool calls: ${toolSuccess}/${NUM_TURNS} (${((toolSuccess / NUM_TURNS) * 100).toFixed(1)}%)`,
  )
  console.log(`Browser disconnects: ${browserDisconnects}`)

  if (screenshotFail > 0 || toolFail > 0 || browserDisconnects > 0) {
    console.log('\n⚠️ Issues detected during long run!')
  } else {
    console.log('\n✅ All operations completed successfully!')
  }
}

main().catch(console.error)
