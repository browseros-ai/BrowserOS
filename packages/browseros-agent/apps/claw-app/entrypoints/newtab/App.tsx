import { HashRouter, Navigate, Route, Routes } from 'react-router'
import { CockpitShell } from '@/components/layout/CockpitShell'
import { Agents } from '@/screens/agents/Agents'
import { Audit } from '@/screens/audit/Audit'
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
import { TaskDetailPage } from '@/screens/task-detail/TaskDetailPage'

const legacyUi = import.meta.env.VITE_COCKPIT_LEGACY_UI === '1'

/** Mounts the cockpit route tree and gates legacy-only surfaces. */
export function App() {
  return (
    <HashRouter>
      <Routes>
        <Route element={<CockpitShell />}>
          <Route path="/" element={<Cockpit />} />
          <Route path="/mcp" element={<Mcp />} />
          <Route path="/audit" element={<Audit />} />
          <Route path="/audit/:sessionId" element={<TaskDetailPage />} />
          <Route path="/audit/:sessionId/replay" element={<Replay />} />
          {legacyUi && (
            <>
              <Route path="/agents" element={<Agents />} />
              <Route path="/agents/new" element={<NewAgent />} />
              <Route
                path="/agents/:id/edit"
                element={<NewAgent mode="edit" />}
              />
              <Route path="/governance" element={<Governance />}>
                <Route index element={<Navigate to="audit" replace />} />
                <Route path="audit" element={<AuditTab />} />
                <Route path="permissions" element={<PermissionsTab />} />
                <Route path="site-rules" element={<SiteRulesTab />} />
                <Route path="grants" element={<GrantsTab />} />
              </Route>
            </>
          )}
        </Route>
        {legacyUi && <Route path="/run/:runId" element={<LiveRun />} />}
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </HashRouter>
  )
}
