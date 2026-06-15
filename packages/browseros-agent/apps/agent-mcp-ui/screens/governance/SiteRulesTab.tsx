import { Lock } from 'lucide-react'
import { ComingSoonTab } from './ComingSoonTab'

export function SiteRulesTab() {
  return (
    <ComingSoonTab
      icon={Lock}
      title="Site Rules"
      description="Per-domain ACL ledger. Browser-enforced blocks that survive prompt injection: payment endpoints, admin panels, anything you never want an agent touching on a given site."
    />
  )
}
