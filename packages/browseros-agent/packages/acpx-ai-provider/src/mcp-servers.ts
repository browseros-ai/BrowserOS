import type { AcpRuntimeOptions } from 'acpx/runtime'
import type { AcpxMcpServerConfig } from './types.ts'

export type RuntimeMcpServer = NonNullable<
  AcpRuntimeOptions['mcpServers']
>[number]

export function toRuntimeMcpServers(
  servers: AcpxMcpServerConfig[] | undefined,
): RuntimeMcpServer[] | undefined {
  if (!servers) return undefined
  return servers.map(toRuntimeMcpServer)
}

function toRuntimeMcpServer(server: AcpxMcpServerConfig): RuntimeMcpServer {
  if (server.type === 'stdio') {
    return {
      name: server.name,
      command: server.command,
      args: server.args ?? [],
      env: recordToEntries(server.env),
    }
  }
  return {
    type: server.type,
    name: server.name,
    url: server.url,
    headers: recordToEntries(server.headers),
  }
}

function recordToEntries(
  record: Record<string, string> | undefined,
): Array<{ name: string; value: string }> {
  if (!record) return []
  return Object.entries(record).map(([name, value]) => ({ name, value }))
}
