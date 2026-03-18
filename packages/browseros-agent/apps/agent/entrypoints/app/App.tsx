import type { FC } from 'react'
import { HashRouter, Navigate, Route, Routes, useParams } from 'react-router'

import { NewTab } from '../newtab/index/NewTab'
import { NewTabChat } from '../newtab/index/NewTabChat'
import { NewTabLayout } from '../newtab/layout/NewTabLayout'
import { Personalize } from '../newtab/personalize/Personalize'
import { OnboardingDemo } from '../onboarding/demo/OnboardingDemo'
import { FeaturesPage } from '../onboarding/features/Features'
import { Onboarding } from '../onboarding/index/Onboarding'
import { StepsLayout } from '../onboarding/steps/StepsLayout'
import { AISettingsPage } from './ai-settings/AISettingsPage'
import { ConnectMCP } from './connect-mcp/ConnectMCP'
import { CreateGraphWrapper } from './create-graph/CreateGraphWrapper'
import { CustomizationPage } from './customization/CustomizationPage'
import { SurveyPage } from './jtbd-agent/SurveyPage'
import { AuthLayout } from './layout/AuthLayout'
import { SettingsSidebarLayout } from './layout/SettingsSidebarLayout'
import { SidebarLayout } from './layout/SidebarLayout'
import { LlmHubPage } from './llm-hub/LlmHubPage'
import { LoginPage } from './login/LoginPage'
import { LogoutPage } from './login/LogoutPage'
import { MagicLinkCallback } from './login/MagicLinkCallback'
import { MCPSettingsPage } from './mcp-settings/MCPSettingsPage'
import { MemoryPage } from './memory/MemoryPage'
import { ProfilePage } from './profile/ProfilePage'
import { ScheduledTasksPage } from './scheduled-tasks/ScheduledTasksPage'
import { SearchProviderPage } from './search-provider/SearchProviderPage'
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

export const App: FC = () => {
  const surveyParams = getSurveyParams()

  return (
    <HashRouter>
      <Routes>
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
            <Route path="chat" element={<NewTabChat />} />
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

        {/* Settings with dedicated sidebar */}
        <Route element={<SettingsSidebarLayout />}>
          <Route path="settings">
            <Route index element={<Navigate to="/settings/ai" replace />} />
            <Route path="ai" element={<AISettingsPage key="ai" />} />
            <Route path="chat" element={<LlmHubPage />} />
            <Route path="mcp" element={<MCPSettingsPage />} />
            <Route path="customization" element={<CustomizationPage />} />
            <Route path="search" element={<SearchProviderPage />} />
            <Route path="survey" element={<SurveyPage {...surveyParams} />} />
          </Route>
        </Route>

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
    </HashRouter>
  )
}
