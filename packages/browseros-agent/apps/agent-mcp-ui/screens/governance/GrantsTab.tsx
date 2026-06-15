import { Check } from 'lucide-react'
import { ComingSoonTab } from './ComingSoonTab'

export function GrantsTab() {
  return (
    <ComingSoonTab
      icon={Check}
      title="Grants"
      description={
        'The always-allow ledger. Every approval you marked "Always allow" rolls up here so you can revoke a standing grant in one click without hunting through a run timeline.'
      }
    />
  )
}
