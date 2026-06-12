import { ShieldCheck } from 'lucide-react'
import { PlaceholderScreen } from '@/components/layout/PlaceholderScreen'

export function Governance() {
  return (
    <PlaceholderScreen
      icon={ShieldCheck}
      title="Governance"
      description="Audit (RR-Web replays of every run), Permissions (safe / ask / blocked buckets), Site Rules (per-domain ACLs), and Grants (the always-allow ledger). The hub the cockpit links into when you need the full picture."
    />
  )
}
