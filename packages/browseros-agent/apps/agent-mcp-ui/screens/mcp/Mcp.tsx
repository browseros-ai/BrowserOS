import { PlugZap } from 'lucide-react'
import { PlaceholderScreen } from '@/components/layout/PlaceholderScreen'

export function Mcp() {
  return (
    <PlaceholderScreen
      icon={PlugZap}
      title="MCP"
      description="The MCP registry for this BrowserOS install. Per-agent endpoints, their slug-routed URLs, the active connection state, and a setup helper for adding BrowserOS as a connector inside Claude, Codex, and friends."
    />
  )
}
