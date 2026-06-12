import { UserPlus } from 'lucide-react'
import { PlaceholderScreen } from '@/components/layout/PlaceholderScreen'

export function NewAgent() {
  return (
    <PlaceholderScreen
      icon={UserPlus}
      title="New profile"
      description="The new-agent wizard. Pick a harness, scope its logins from an imported Chrome profile, set bucket overrides for the tool surface, layer ACL rules, and get back a copyable MCP URL ready to paste into the harness."
    />
  )
}
