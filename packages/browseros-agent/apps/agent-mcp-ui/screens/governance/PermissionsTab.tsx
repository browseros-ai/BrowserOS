import { ShieldCheck } from 'lucide-react'
import { ComingSoonTab } from './ComingSoonTab'

export function PermissionsTab() {
  return (
    <ComingSoonTab
      icon={ShieldCheck}
      title="Permissions"
      description="The 3-bucket action catalog (safe / ask / blocked) every agent inherits from. Drag rules between buckets to widen or tighten the default surface for all agents at once."
    />
  )
}
