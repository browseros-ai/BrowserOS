import { HashRouter, Navigate, Route, Routes } from 'react-router'
import { CockpitShell } from '@/components/layout/CockpitShell'
import { Agents } from '@/screens/agents/Agents'
import { Cockpit } from '@/screens/cockpit/Cockpit'
import { AuditTab } from '@/screens/governance/AuditTab'
import { Governance } from '@/screens/governance/Governance'
import { GrantsTab } from '@/screens/governance/GrantsTab'
import { PermissionsTab } from '@/screens/governance/PermissionsTab'
import { SiteRulesTab } from '@/screens/governance/SiteRulesTab'
import { LiveRun } from '@/screens/live-run/LiveRun'
import { Mcp } from '@/screens/mcp/Mcp'
import { NewAgent } from '@/screens/new-agent/NewAgent'
import { Replay } from '@/screens/replay/Replay'

/**
 * HashRouter wrapping a single layout route that mounts the sidebar
 * plus main outlet for every screen. Governance is a nested layout
 * route so its tab bar stays mounted while the URL drives which tab
 * panel renders.
 */
export function App() {
  return (
    <HashRouter>
      <Routes>
        <Route element={<CockpitShell />}>
          <Route path="/" element={<Cockpit />} />
          <Route path="/agents" element={<Agents />} />
          <Route path="/agents/new" element={<NewAgent />} />
          <Route path="/governance" element={<Governance />}>
            <Route index element={<Navigate to="audit" replace />} />
            <Route path="audit" element={<AuditTab />} />
            <Route path="permissions" element={<PermissionsTab />} />
            <Route path="site-rules" element={<SiteRulesTab />} />
            <Route path="grants" element={<GrantsTab />} />
          </Route>
          <Route path="/mcp" element={<Mcp />} />
        </Route>
        <Route path="/run/:runId" element={<LiveRun />} />
        <Route path="/governance/audit/:runId/replay" element={<Replay />} />
      </Routes>
    </HashRouter>
  )
}
