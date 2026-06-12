import { HashRouter, Route, Routes } from 'react-router'
import { CockpitShell } from '@/components/layout/CockpitShell'
import { Agents } from '@/screens/agents/Agents'
import { Cockpit } from '@/screens/cockpit/Cockpit'
import { Governance } from '@/screens/governance/Governance'
import { Mcp } from '@/screens/mcp/Mcp'
import { NewAgent } from '@/screens/new-agent/NewAgent'

/**
 * HashRouter wrapping a single layout route that mounts the sidebar
 * plus main outlet for every screen. The cockpit (/) is the home;
 * the three other primary nav routes plus the new-agent wizard route
 * sit as placeholders until their own slices land.
 */
export function App() {
  return (
    <HashRouter>
      <Routes>
        <Route element={<CockpitShell />}>
          <Route path="/" element={<Cockpit />} />
          <Route path="/agents" element={<Agents />} />
          <Route path="/agents/new" element={<NewAgent />} />
          <Route path="/governance" element={<Governance />} />
          <Route path="/mcp" element={<Mcp />} />
        </Route>
      </Routes>
    </HashRouter>
  )
}
