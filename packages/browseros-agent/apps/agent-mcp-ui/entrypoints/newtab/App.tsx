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
import { Onboarding } from '@/screens/onboarding/Onboarding'
import { OnboardingV2 } from '@/screens/onboarding/OnboardingV2'
import { Replay } from '@/screens/replay/Replay'

/**
 * HashRouter wrapping a single layout route that mounts the sidebar
 * plus main outlet for every screen. Governance is a nested layout
 * route so its tab bar stays mounted while the URL drives which tab
 * panel renders.
 *
 * Legacy multi-agent surfaces (agents directory, governance, replay,
 * the wizard-driven onboarding) are gated behind
 * `VITE_COCKPIT_LEGACY_UI=1`. v2 ships the homepage, the MCP page,
 * and a placeholder onboarding stub; everything else is removed from
 * the route tree so stale links redirect home rather than rendering
 * a deprecated screen.
 */
const legacyUi = import.meta.env.VITE_COCKPIT_LEGACY_UI === '1'

export function App() {
  return (
    <HashRouter>
      <Routes>
        <Route element={<CockpitShell />}>
          <Route path="/" element={<Cockpit />} />
          <Route path="/mcp" element={<Mcp />} />
          <Route path="/audit" element={<Audit />} />
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
        {legacyUi && (
          <>
            <Route path="/run/:runId" element={<LiveRun />} />
            <Route
              path="/governance/audit/:runId/replay"
              element={<Replay />}
            />
            <Route path="/onboarding" element={<Onboarding />} />
          </>
        )}
        {!legacyUi && <Route path="/onboarding" element={<OnboardingV2 />} />}
        {/* Catch-all: any unregistered route lands on the homepage so
            a stale link from before the v2 cut does not render a 404. */}
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </HashRouter>
  )
}
