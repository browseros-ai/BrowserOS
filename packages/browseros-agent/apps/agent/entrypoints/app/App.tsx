import type { FC } from 'react'
import {
  HashRouter,
  type Location,
  Navigate,
  Route,
  Routes,
  useLocation,
  useParams,
} from 'react-router'

import { NewTab } from '../newtab/index/NewTab'
import { NewTabLayout } from '../newtab/layout/NewTabLayout'
import { Personalize } from '../newtab/personalize/Personalize'
import { OnboardingDemo } from '../onboarding/demo/OnboardingDemo'
import { FeaturesPage } from '../onboarding/features/Features'
import { Onboarding } from '../onboarding/index/Onboarding'
import { StepsLayout } from '../onboarding/steps/StepsLayout'
import { ConnectMCP } from './connect-mcp/ConnectMCP'
import { CreateGraphWrapper } from './create-graph/CreateGraphWrapper'
import { SurveyPage } from './jtbd-agent/SurveyPage'
import { AuthLayout } from './layout/AuthLayout'
import { SidebarLayout } from './layout/SidebarLayout'
import { LoginPage } from './login/LoginPage'
import { LogoutPage } from './login/LogoutPage'
import { MagicLinkCallback } from './login/MagicLinkCallback'
import { MemoryPage } from './memory/MemoryPage'
import { ProfilePage } from './profile/ProfilePage'
import { ScheduledTasksPage } from './scheduled-tasks/ScheduledTasksPage'
import { SettingsDialog } from './settings-dialog/SettingsDialog'
import { SkillsPage } from './skills/SkillsPage'
import { SoulPage } from './soul/SoulPage'
import { WorkflowsPageWrapper } from './workflows/WorkflowsPageWrapper'

function getSurveyParams(): { maxTurns?: number; experimentId?: string } {
  const params = new URLSearchParams(window.location.search)
  const maxTurnsStr = params.get('maxTurns')
  const experimentId = params.get('experimentId') ?? 'default'
  const maxTurns = maxTurnsStr ? Number.parseInt(maxTurnsStr, 10) : 7
  return { maxTurns, experimentId }
}

const OptionsRedirect: FC = () => {
  const params = useParams()
  const path = params['*'] || ''

  const routeMap: Record<string, string> = {
    ai: '/settings/ai',
    chat: '/settings/chat',
    'connect-mcp': '/connect-apps',
    mcp: '/settings/mcp',
    customization: '/settings/customization',
    search: '/settings/search',
    soul: '/home/soul',
    skills: '/home/skills',
    'jtbd-agent': '/settings/survey',
    workflows: '/workflows',
    scheduled: '/scheduled',
    'create-graph': '/workflows/create-graph',
  }

  const newPath = routeMap[path] || '/settings/ai'
  return <Navigate to={newPath} replace />
}

/** Redirect direct /settings/:tab visits so the dialog has a background page */
const SettingsRedirect: FC = () => {
  const { tab } = useParams()
  return (
    <Navigate
      to={`/settings/${tab || 'ai'}`}
      state={{ backgroundLocation: { pathname: '/home' } }}
      replace
    />
  )
}

const AppRoutes: FC = () => {
  const location = useLocation()
  const surveyParams = getSurveyParams()

  const backgroundLocation = (
    location.state as { backgroundLocation?: Location } | null
  )?.backgroundLocation

  return (
    <>
      <Routes location={backgroundLocation || location}>
        {/* Public auth routes */}
        <Route element={<AuthLayout />}>
          <Route path="login" element={<LoginPage />} />
          <Route path="logout" element={<LogoutPage />} />
          <Route path="profile" element={<ProfilePage />} />
          <Route path="auth/magic-link" element={<MagicLinkCallback />} />
        </Route>

        {/* Main app with sidebar */}
        <Route element={<SidebarLayout />}>
          {/* Home routes */}
          <Route path="home" element={<NewTabLayout />}>
            <Route index element={<NewTab />} />
            <Route path="personalize" element={<Personalize />} />
            <Route path="soul" element={<SoulPage />} />
            <Route path="skills" element={<SkillsPage />} />
            <Route path="memory" element={<MemoryPage />} />
          </Route>

          {/* Primary nav routes */}
          <Route path="connect-apps" element={<ConnectMCP />} />
          <Route path="workflows" element={<WorkflowsPageWrapper />} />
          <Route path="scheduled" element={<ScheduledTasksPage />} />
        </Route>

        {/* Survey page - standalone */}
        <Route
          path="settings/survey"
          element={<SurveyPage {...surveyParams} />}
        />

        {/* Direct /settings/:tab access without background location — redirect with one */}
        <Route path="settings/:tab?" element={<SettingsRedirect />} />

        {/* Full-screen without sidebar */}
        <Route path="workflows/create-graph" element={<CreateGraphWrapper />} />

        {/* Onboarding routes - no sidebar, no auth required */}
        <Route path="onboarding">
          <Route index element={<Onboarding />} />
          <Route path="steps/:stepId" element={<StepsLayout />} />
          <Route path="demo" element={<OnboardingDemo />} />
          <Route path="features" element={<FeaturesPage />} />
        </Route>

        {/* Backward compatibility redirects */}
        <Route path="/" element={<Navigate to="/home" replace />} />
        <Route
          path="/personalize"
          element={<Navigate to="/home/personalize" replace />}
        />
        <Route
          path="/settings/connect-mcp"
          element={<Navigate to="/connect-apps" replace />}
        />
        <Route
          path="/settings/soul"
          element={<Navigate to="/home/soul" replace />}
        />
        <Route
          path="/settings/skills"
          element={<Navigate to="/home/skills" replace />}
        />
        <Route path="/options/*" element={<OptionsRedirect />} />

        {/* Fallback to home */}
        <Route path="*" element={<Navigate to="/home" replace />} />
      </Routes>

      {/* Modal overlay — renders settings dialog on top of background page */}
      {backgroundLocation && (
        <Routes>
          <Route path="settings/:tab?" element={<SettingsDialog />} />
        </Routes>
      )}
    </>
  )
}

export const App: FC = () => (
  <HashRouter>
    <AppRoutes />
  </HashRouter>
)
