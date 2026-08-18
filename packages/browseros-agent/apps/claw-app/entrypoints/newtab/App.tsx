import type { ReactNode } from 'react'
import { HashRouter, Navigate, Route, Routes } from 'react-router'
import { CockpitShell } from '@/components/layout/CockpitShell'
import { Analytics } from '@/modules/analytics/Analytics'
import { Audit } from '@/screens/audit/Audit'
import { Cockpit } from '@/screens/cockpit/Cockpit'
import { Mcp } from '@/screens/mcp/Mcp'
import { Replay } from '@/screens/replay/Replay'
import { SkillDetail } from '@/screens/skills/SkillDetail'
import { Skills } from '@/screens/skills/Skills'
import { TaskDetailPage } from '@/screens/task-detail/TaskDetailPage'

/** Mounts the v2 cockpit route tree. */
export function App() {
  return (
    <HashRouter>
      <Analytics />
      <Routes>
        <Route element={<CockpitShell />}>
          <Route path="/" element={<Cockpit />} />
          <Route path="/mcp" element={<Mcp />} />
          <Route path="/skills" element={<Skills />} />
          <Route path="/skills/:name" element={<SkillDetail />} />
          <Route
            path="/audit"
            element={
              <SensitiveRouteContent>
                <Audit />
              </SensitiveRouteContent>
            }
          />
          <Route
            path="/audit/:sessionId"
            element={
              <SensitiveRouteContent>
                <TaskDetailPage />
              </SensitiveRouteContent>
            }
          />
          <Route
            path="/audit/:sessionId/replay"
            element={
              <SensitiveRouteContent>
                <Replay />
              </SensitiveRouteContent>
            }
          />
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </HashRouter>
  )
}

/** Blocks task-bearing route bodies from PostHog session snapshots. */
export function SensitiveRouteContent({ children }: { children: ReactNode }) {
  return <div className="ph-no-capture contents">{children}</div>
}
