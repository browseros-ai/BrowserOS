/**
 * OpenClaw Bridge — ส่วนเชื่อมต่อเฉพาะสำหรับ OpenClaw
 *
 * ให้ helper functions สำหรับ:
 *   - สร้าง OpenClaw config สำหรับ MCP server
 *   - ทดสอบการเชื่อมต่อกับ headless server
 *   - จัดการ session lifecycle สำหรับ OpenClaw context
 */

import { randomUUID } from 'crypto'

export interface OpenClawMCPConfig {
  /** ชื่อ MCP server ใน OpenClaw config */
  name: string
  /** Transport type */
  transport: 'stdio' | 'sse'
  /** คำสั่งสำหรับ stdio mode */
  command?: string
  /** args สำหรับ stdio mode */
  args?: string[]
  /** URL สำหรับ SSE mode */
  url?: string
  /** environment variables */
  env?: Record<string, string>
}

/** สร้าง OpenClaw MCP config สำหรับ stdio transport */
export function createStdioConfig(options?: {
  headlessUrl?: string
  apiKey?: string
}): OpenClawMCPConfig {
  return {
    name: 'browseros',
    transport: 'stdio',
    command: 'bun',
    args: ['run', 'packages/browseros-agent/apps/headless-server/mcp-server.ts'],
    env: {
      HEADLESS_SERVER_URL: options?.headlessUrl || 'http://127.0.0.1:3100',
      HEADLESS_API_KEY: options?.apiKey || '',
    },
  }
}

/** สร้าง OpenClaw MCP config สำหรับ SSE transport */
export function createSSEConfig(options?: {
  mcpUrl?: string
}): OpenClawMCPConfig {
  return {
    name: 'browseros',
    transport: 'sse',
    url: options?.mcpUrl || 'http://127.0.0.1:3200/sse',
  }
}

/** ทดสอบการเชื่อมต่อกับ headless server */
export async function testHeadlessConnection(
  url: string = 'http://127.0.0.1:3100',
): Promise<{ ok: boolean; info?: any; error?: string }> {
  try {
    const resp = await fetch(`${url}/api/status`)
    const data = await resp.json()
    return { ok: true, info: data }
  } catch (err: any) {
    return { ok: false, error: err.message }
  }
}

/** สร้าง OpenClaw JSON config string สำหรับวางใน .openclaw/openclaw.json */
export function generateOpenClawConfig(options?: {
  transport?: 'stdio' | 'sse'
  headlessUrl?: string
  mcpUrl?: string
  apiKey?: string
}): string {
  const transport = options?.transport || 'stdio'
  const config = transport === 'sse'
    ? createSSEConfig({ mcpUrl: options?.mcpUrl })
    : createStdioConfig({ headlessUrl: options?.headlessUrl, apiKey: options?.apiKey })

  const mcpEntry: Record<string, unknown> = {
    [config.name]: {
      transport: config.transport,
    },
  }

  if (config.transport === 'stdio') {
    mcpEntry[config.name] = {
      ...mcpEntry[config.name],
      command: config.command,
      args: config.args,
      env: config.env,
    }
  } else {
    mcpEntry[config.name] = {
      ...mcpEntry[config.name],
      url: config.url,
    }
  }

  return JSON.stringify({ mcp: { servers: mcpEntry } }, null, 2)
}

// ─── CLI utility ──────────────────────────────────────────
// ถ้ารันไฟล์นี้ตรง จะพิมพ์ config ตัวอย่าง
const isMainModule = process.argv[1]?.includes('openclaw-bridge')
if (isMainModule) {
  const transport = process.argv[2] || 'stdio'
  console.log('// ตัวอย่าง config สำหรับ .openclaw/openclaw.json')
  console.log(generateOpenClawConfig({ transport: transport as 'stdio' | 'sse' }))
}
