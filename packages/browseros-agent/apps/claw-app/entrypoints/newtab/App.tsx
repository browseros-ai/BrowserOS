import { DiagnosticsPage } from '@browseros/diagnostics/view'
import type { ReactNode } from 'react'
import { HashRouter, Navigate, Route, Routes } from 'react-router'
import { AppLayout } from '@/components/layout/AppLayout'
import { Page } from '@/components/layout/Page'
import { Analytics } from '@/modules/analytics/Analytics'
import { Audit } from '@/screens/audit/Audit'
import { Cockpit } from '@/screens/cockpit/Cockpit'
import { Mcp } from '@/screens/mcp/Mcp'
import { Replay } from '@/screens/replay/Replay'
import { Settings } from '@/screens/settings/Settings'
import { SkillDetail } from '@/screens/skills/SkillDetail'
import { Skills } from '@/screens/skills/Skills'
import { TaskDetailPage } from '@/screens/task-detail/TaskDetailPage'
import { UltrafastWaitlist } from '@/screens/ultrafast/UltrafastWaitlist'

/** Mounts the v2 cockpit route tree. */
export function App() {
  return (
    <HashRouter>
      <Analytics />
      <Routes>
        <Route element={<AppLayout />}>
          <Route path="/" element={<Cockpit />} />
          <Route path="/mcp" element={<Mcp />} />
          <Route
            path="/diagnostics"
            element={
              <Page>
                <DiagnosticsPage />
              </Page>
            }
          />
          <Route path="/skills" element={<Skills />} />
          <Route path="/skills/:name" element={<SkillDetail />} />
          <Route path="/ultrafast" element={<UltrafastWaitlist />} />
          <Route path="/settings" element={<Settings />} />
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
