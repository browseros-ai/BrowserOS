/**
 * Test script to validate the complete eval lifecycle
 * Run with: bun apps/eval/scripts/test-lifecycle.ts
 *
 * Tests:
 * 1. BrowserOS app detection
 * 2. Server start/stop
 * 3. Browser readiness with verification
 * 4. Window create/close
 * 5. Screenshot capture
 * 6. Multiple tasks in sequence with server restart
 */

import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { type Subprocess, spawn, spawnSync } from 'bun'

// Ports from config.dev.json - must match BrowserOS launch args
const EVAL_PORTS = {
  cdp: 9005,
  server: 9105, // http_mcp in config.dev.json
} as const
const MONOREPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '../../..')
const MCP_URL = `http://127.0.0.1:${EVAL_PORTS.server}/mcp`

let currentServerPid: number | null = null

// ============================================================================
// Utility Functions (same as parallel-executor)
// ============================================================================

function killPort(port: number): void {
  spawnSync({
    cmd: ['sh', '-c', `lsof -ti:${port} | xargs kill -9 2>/dev/null || true`],
  })
}

function isBrowserOSAppRunning(): boolean {
  const result = spawnSync({
    cmd: ['sh', '-c', 'pgrep -f "BrowserOS" 2>/dev/null || true'],
  })
  const output = result.stdout?.toString().trim() ?? ''
  return output.length > 0
}

async function _killBrowserOSApp(): Promise<void> {
  console.log('  Killing BrowserOS app...')
  spawnSync({
    cmd: ['sh', '-c', 'pkill -9 -f "BrowserOS" 2>/dev/null || true'],
  })
  killPort(EVAL_PORTS.cdp)
  for (let i = 0; i < 10; i++) {
    if (!isBrowserOSAppRunning()) return
    await new Promise((r) => setTimeout(r, 500))
  }
}

async function _launchBrowserOSApp(): Promise<boolean> {
  console.log(
    `  Launching BrowserOS (server disabled, CDP=${EVAL_PORTS.cdp})...`,
  )
  spawnSync({
    cmd: [
      'open',
      '-a',
      'BrowserOS',
      '--args',
      '--disable-browseros-server',
      `--remote-debugging-port=${EVAL_PORTS.cdp}`,
      `--browseros-cdp-port=${EVAL_PORTS.cdp}`,
      `--browseros-mcp-port=${EVAL_PORTS.server}`,
    ],
  })
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 1000))
    if (isBrowserOSAppRunning()) {
      await new Promise((r) => setTimeout(r, 8000))
      return true
    }
  }
  return false
}

async function waitForPortFree(
  port: number,
  maxAttempts = 30,
): Promise<boolean> {
  for (let i = 0; i < maxAttempts; i++) {
    const result = spawnSync({
      cmd: ['sh', '-c', `lsof -ti:${port} 2>/dev/null`],
    })
    if (!result.stdout || result.stdout.toString().trim() === '') {
      return true
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  return false
}

async function waitForServerHealth(
  serverPort: number,
  maxAttempts = 60,
): Promise<boolean> {
  for (let i = 0; i < maxAttempts; i++) {
    try {
      const response = await fetch(`http://127.0.0.1:${serverPort}/health`, {
        signal: AbortSignal.timeout(1000),
      })
      if (response.ok) return true
    } catch {
      /* not ready */
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  return false
}

async function waitForBrowserReady(
  serverPort: number,
  maxAttempts = 90,
): Promise<boolean> {
  let connectedCount = 0
  for (let i = 0; i < maxAttempts; i++) {
    try {
      const response = await fetch(`http://127.0.0.1:${serverPort}/health`, {
        signal: AbortSignal.timeout(2000),
      })
      if (response.ok) {
        const data = (await response.json()) as { cdpConnected?: boolean }
        if (data.cdpConnected) {
          connectedCount++
          if (connectedCount >= 3) return true
        } else {
          connectedCount = 0
        }
      }
    } catch {
      connectedCount = 0
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  return false
}

async function startServer(): Promise<Subprocess> {
  killPort(EVAL_PORTS.server)
  await waitForPortFree(EVAL_PORTS.server, 30)

  const serverProc = spawn({
    cmd: [
      'bun',
      'apps/server/src/index.ts',
      '--server-port',
      String(EVAL_PORTS.server),
      '--cdp-port',
      String(EVAL_PORTS.cdp),
    ],
    cwd: MONOREPO_ROOT,
    stdout: 'pipe',
    stderr: 'pipe',
    env: { ...process.env, NODE_ENV: 'development' },
  })

  currentServerPid = serverProc.pid
  return serverProc
}

async function stopServer(proc: Subprocess): Promise<void> {
  try {
    proc.kill('SIGKILL')
    await Promise.race([
      proc.exited,
      new Promise((resolve) => setTimeout(resolve, 5000)),
    ])
  } catch {
    /* ignore */
  }
  currentServerPid = null
}

async function callMcpTool(
  name: string,
  args: Record<string, unknown> = {},
  timeoutMs = 60000,
): Promise<{ success: boolean; result?: any; error?: string }> {
  const client = new Client({ name: 'lifecycle-test', version: '1.0.0' })
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

    if ((result as any).isError) {
      const errorText =
        (result as any).content?.find((c: any) => c.type === 'text')?.text ||
        'Unknown error'
      return { success: false, error: errorText }
    }
    return { success: true, result }
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : String(error),
    }
  } finally {
    try {
      await transport.close()
    } catch {}
  }
}

// ============================================================================
// Tests
// ============================================================================

async function testBrowserOSDetection(): Promise<boolean> {
  console.log('\n=== Test 1: BrowserOS App Detection ===')
  const running = isBrowserOSAppRunning()
  console.log(`  BrowserOS running: ${running}`)
  if (!running) {
    console.log('  ❌ BrowserOS app is not running. Please start it.')
    return false
  }
  console.log('  ✅ BrowserOS app detected')
  return true
}

async function testServerStartStop(): Promise<boolean> {
  console.log('\n=== Test 2: Server Start/Stop ===')

  console.log('  Starting server...')
  const proc = await startServer()
  console.log(`  Server PID: ${proc.pid}`)

  console.log('  Waiting for health...')
  const healthy = await waitForServerHealth(EVAL_PORTS.server, 30)
  if (!healthy) {
    console.log('  ❌ Server health check failed')
    await stopServer(proc)
    return false
  }
  console.log('  ✅ Server healthy')

  console.log('  Waiting for browser readiness...')
  const browserReady = await waitForBrowserReady(EVAL_PORTS.server, 60)
  if (!browserReady) {
    console.log('  ❌ Browser did not become ready')
    await stopServer(proc)
    return false
  }
  console.log('  ✅ Browser ready')

  console.log('  Stopping server...')
  await stopServer(proc)
  console.log('  ✅ Server stopped')

  return true
}

async function testWindowLifecycle(): Promise<boolean> {
  console.log('\n=== Test 3: Window Create/Close ===')

  console.log('  Starting server...')
  const proc = await startServer()

  const healthy = await waitForServerHealth(EVAL_PORTS.server, 30)
  if (!healthy) {
    console.log('  ❌ Server health check failed')
    await stopServer(proc)
    return false
  }

  const browserReady = await waitForBrowserReady(EVAL_PORTS.server, 60)
  if (!browserReady) {
    console.log('  ❌ Browser did not become ready')
    await stopServer(proc)
    return false
  }

  console.log('  Creating window...')
  const createResult = await callMcpTool('browser_create_window', {
    url: 'https://example.com',
    focused: false,
  })

  if (!createResult.success) {
    console.log(`  ❌ Failed to create window: ${createResult.error}`)
    await stopServer(proc)
    return false
  }

  const windowId = createResult.result?.structuredContent?.windowId
  const tabId = createResult.result?.structuredContent?.tabId
  console.log(`  ✅ Window created: windowId=${windowId}, tabId=${tabId}`)

  // Wait for page load
  await new Promise((r) => setTimeout(r, 2000))

  // Take screenshot
  console.log('  Taking screenshot...')
  const ssResult = await callMcpTool('browser_get_screenshot', {
    tabId,
    windowId,
    size: 'small',
  })

  if (!ssResult.success) {
    console.log(`  ❌ Screenshot failed: ${ssResult.error}`)
  } else {
    console.log('  ✅ Screenshot captured')
  }

  // Close window
  console.log('  Closing window...')
  const closeResult = await callMcpTool('browser_close_window', { windowId })
  if (!closeResult.success) {
    console.log(
      `  ⚠️  Close window returned error (may be expected): ${closeResult.error}`,
    )
  } else {
    console.log('  ✅ Window closed')
  }

  console.log('  Stopping server...')
  await stopServer(proc)
  console.log('  ✅ Server stopped')

  return true
}

async function testMultipleTasksWithRestart(): Promise<boolean> {
  console.log('\n=== Test 4: Multiple Tasks with Server Restart ===')

  const tasks = [
    { id: 'task-1', url: 'https://example.com' },
    { id: 'task-2', url: 'https://google.com' },
    { id: 'task-3', url: 'https://github.com' },
  ]

  let successCount = 0

  for (const task of tasks) {
    console.log(`\n  --- Task: ${task.id} ---`)

    // Start server
    console.log('  Starting server...')
    const proc = await startServer()

    const healthy = await waitForServerHealth(EVAL_PORTS.server, 30)
    if (!healthy) {
      console.log(`  ❌ Task ${task.id}: Server health failed`)
      await stopServer(proc)
      continue
    }

    const browserReady = await waitForBrowserReady(EVAL_PORTS.server, 60)
    if (!browserReady) {
      console.log(`  ❌ Task ${task.id}: Browser not ready`)
      await stopServer(proc)
      continue
    }

    // Create window
    const createResult = await callMcpTool('browser_create_window', {
      url: task.url,
      focused: false,
    })

    if (!createResult.success) {
      console.log(
        `  ❌ Task ${task.id}: Window creation failed - ${createResult.error}`,
      )
      await stopServer(proc)
      continue
    }

    const windowId = createResult.result?.structuredContent?.windowId
    console.log(`  Window created: ${windowId}`)

    await new Promise((r) => setTimeout(r, 2000))

    // Close window
    await callMcpTool('browser_close_window', { windowId })
    console.log(`  Window closed`)

    // Stop server
    await stopServer(proc)
    console.log(`  Server stopped`)

    successCount++
    console.log(`  ✅ Task ${task.id} completed`)

    // Delay between tasks
    await new Promise((r) => setTimeout(r, 2000))
  }

  console.log(`\n  Results: ${successCount}/${tasks.length} tasks successful`)
  return successCount === tasks.length
}

async function testBrowserStability(): Promise<boolean> {
  console.log('\n=== Test 5: Browser Stability (30 seconds) ===')

  console.log('  Starting server...')
  const proc = await startServer()

  const healthy = await waitForServerHealth(EVAL_PORTS.server, 30)
  if (!healthy) {
    console.log('  ❌ Server health check failed')
    await stopServer(proc)
    return false
  }

  const browserReady = await waitForBrowserReady(EVAL_PORTS.server, 60)
  if (!browserReady) {
    console.log('  ❌ Browser did not become ready')
    await stopServer(proc)
    return false
  }

  console.log('  Monitoring browser readiness for 30 seconds...')
  let disconnects = 0
  const checkInterval = 2000
  const totalChecks = 30000 / checkInterval

  for (let i = 0; i < totalChecks; i++) {
    try {
      const response = await fetch(
        `http://127.0.0.1:${EVAL_PORTS.server}/health`,
        {
          signal: AbortSignal.timeout(2000),
        },
      )
      const data = (await response.json()) as { cdpConnected?: boolean }
      if (!data.cdpConnected) {
        disconnects++
        console.log(
          `  ⚠️  Browser became unavailable at check ${i + 1}/${totalChecks}`,
        )
      }
    } catch {
      disconnects++
      console.log(`  ⚠️  Failed to check browser at ${i + 1}/${totalChecks}`)
    }
    await new Promise((r) => setTimeout(r, checkInterval))
  }

  await stopServer(proc)

  if (disconnects > 0) {
    console.log(`  ❌ Browser had ${disconnects} readiness failures`)
    return false
  }

  console.log('  ✅ Browser stayed ready for 30 seconds')
  return true
}

// ============================================================================
// Main
// ============================================================================

async function main() {
  console.log('='.repeat(60))
  console.log('Eval Lifecycle Test Suite')
  console.log('='.repeat(60))
  console.log(`Server Port: ${EVAL_PORTS.server}`)
  console.log(`CDP Port: ${EVAL_PORTS.cdp}`)

  const results: { name: string; passed: boolean }[] = []

  // Test 1: BrowserOS Detection
  results.push({
    name: 'BrowserOS Detection',
    passed: await testBrowserOSDetection(),
  })
  if (!results[0].passed) {
    console.log('\n❌ Cannot continue without BrowserOS app running')
    process.exit(1)
  }

  // Test 2: Server Start/Stop
  results.push({
    name: 'Server Start/Stop',
    passed: await testServerStartStop(),
  })

  // Test 3: Window Lifecycle
  results.push({
    name: 'Window Lifecycle',
    passed: await testWindowLifecycle(),
  })

  // Test 4: Multiple Tasks
  results.push({
    name: 'Multiple Tasks',
    passed: await testMultipleTasksWithRestart(),
  })

  // Test 5: Browser Stability
  results.push({
    name: 'Browser Stability',
    passed: await testBrowserStability(),
  })

  // Summary
  console.log(`\n${'='.repeat(60)}`)
  console.log('SUMMARY')
  console.log('='.repeat(60))

  const passed = results.filter((r) => r.passed).length
  const failed = results.filter((r) => !r.passed).length

  for (const r of results) {
    console.log(`  ${r.passed ? '✅' : '❌'} ${r.name}`)
  }

  console.log(`\nTotal: ${passed} passed, ${failed} failed`)

  if (failed > 0) {
    process.exit(1)
  }
}

main().catch((error) => {
  console.error('Test suite failed:', error)
  if (currentServerPid) {
    try {
      process.kill(currentServerPid, 'SIGKILL')
    } catch {}
  }
  process.exit(1)
})
