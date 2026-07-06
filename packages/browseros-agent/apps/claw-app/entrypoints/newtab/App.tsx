import { HashRouter, Navigate, Route, Routes } from 'react-router'
import { CockpitShell } from '@/components/layout/CockpitShell'
import { Audit } from '@/screens/audit/Audit'
import { Cockpit } from '@/screens/cockpit/Cockpit'
import { Mcp } from '@/screens/mcp/Mcp'
import { Replay } from '@/screens/replay/Replay'
import { TaskDetailPage } from '@/screens/task-detail/TaskDetailPage'

/** Mounts the v2 cockpit route tree. */
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
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </HashRouter>
  )
}
