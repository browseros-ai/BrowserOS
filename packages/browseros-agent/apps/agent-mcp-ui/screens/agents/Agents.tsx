import { Bot } from 'lucide-react'
import { PlaceholderScreen } from '@/components/layout/PlaceholderScreen'

export function Agents() {
  return (
    <PlaceholderScreen
      icon={Bot}
      title="Agents"
      description="The full agents directory lives here. From this screen you'll connect new agent profiles, manage their login scope and ACL rules, and see which harnesses are currently registered against BrowserOS."
    />
  )
}
