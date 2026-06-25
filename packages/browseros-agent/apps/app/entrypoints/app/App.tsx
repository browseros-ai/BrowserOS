import type { FC } from 'react'
import { HashRouter, Navigate, Route, Routes, useParams } from 'react-router'
import { AuthLayout } from '@/components/layout/AuthLayout'
import { SettingsSidebarLayout } from '@/components/layout/SettingsSidebarLayout'
import { SidebarLayout } from '@/components/layout/SidebarLayout'
import { AgentCommandConversation } from '@/screens/agent-command/AgentCommandConversation'
import { AgentCommandHome } from '@/screens/agent-command/AgentCommandHome'
import { AgentCommandLayout } from '@/screens/agent-command/AgentCommandLayout'
import { AISettingsPage } from '@/screens/ai-settings/AISettingsPage'
import { LoginPage } from '@/screens/auth/LoginPage'
import { LogoutPage } from '@/screens/auth/LogoutPage'
import { ConnectMCP } from '@/screens/connect-mcp/ConnectMCP'
import { CustomizationPage } from '@/screens/customization/CustomizationPage'
import { SurveyPage } from '@/screens/jtbd-agent/SurveyPage'
import { LlmHubPage } from '@/screens/llm-hub/LlmHubPage'
import { MCPSettingsPage } from '@/screens/mcp-settings/MCPSettingsPage'
import { NewTabChat } from '@/screens/newtab/index/NewTabChat'
import { NewTabLayout } from '@/screens/newtab/layout/NewTabLayout'
import { Personalize } from '@/screens/newtab/personalize/Personalize'
import { OnboardingDemo } from '@/screens/onboarding/demo/OnboardingDemo'
import { FeaturesPage } from '@/screens/onboarding/features/Features'
import { Onboarding } from '@/screens/onboarding/index/Onboarding'
import { StepsLayout } from '@/screens/onboarding/steps/StepsLayout'
import { ProfilePage } from '@/screens/profile/ProfilePage'
import { ScheduledTasksPage } from '@/screens/scheduled-tasks/ScheduledTasksPage'
import { UsagePage } from '@/screens/usage/UsagePage'

function getSurveyParams(): { maxTurns?: number; experimentId?: string } {
  const params = new URLSearchParams(window.location.search)
  const maxTurnsStr = params.get('maxTurns')
  const experimentId = params.get('experimentId') ?? 'default'
  const maxTurns = maxTurnsStr ? Number.parseInt(maxTurnsStr, 10) : 7
  return { maxTurns, experimentId }
}

// Agent management moved into AI & Agents settings; conversations live under
// /home/agents. Keep old /agents links alive.
const LegacyAgentRedirect: FC = () => {
  const params = useParams()
  return <Navigate to={`/home/agents/${params.agentId ?? ''}`} replace />
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
    search: '/settings/ai',
    'jtbd-agent': '/settings/survey',
    scheduled: '/scheduled',
  }

  const newPath = routeMap[path] || '/settings/ai'
  return <Navigate to={newPath} replace />
}

export const App: FC = () => {
  const surveyParams = getSurveyParams()

  return (
    <HashRouter>
      <Routes>
        <Route element={<AuthLayout />}>
          <Route path="login" element={<LoginPage />} />
          <Route path="logout" element={<LogoutPage />} />
          <Route path="profile" element={<ProfilePage />} />
        </Route>

        <Route element={<SidebarLayout />}>
          <Route path="home" element={<NewTabLayout />}>
            <Route element={<AgentCommandLayout />}>
              <Route index element={<AgentCommandHome />} />
              <Route
                path="agents/:agentId"
                element={<AgentCommandConversation />}
              />
              <Route
                path="agents/:agentId/sessions/:sessionId"
                element={<AgentCommandConversation />}
              />
            </Route>
            <Route path="chat" element={<NewTabChat />} />
            <Route path="personalize" element={<Personalize />} />
          </Route>

          <Route path="connect-apps" element={<ConnectMCP />} />
          <Route path="scheduled" element={<ScheduledTasksPage />} />
        </Route>

        <Route element={<SettingsSidebarLayout />}>
          <Route path="settings">
            <Route index element={<Navigate to="/settings/ai" replace />} />
            <Route path="ai" element={<AISettingsPage key="ai" />} />
            <Route path="chat" element={<LlmHubPage />} />
            <Route path="mcp" element={<MCPSettingsPage />} />
            <Route path="customization" element={<CustomizationPage />} />
            <Route
              path="search"
              element={<Navigate to="/settings/ai" replace />}
            />
            <Route path="survey" element={<SurveyPage {...surveyParams} />} />
            <Route path="usage" element={<UsagePage />} />
            <Route path="*" element={<Navigate to="/settings/ai" replace />} />
          </Route>
        </Route>

        <Route path="onboarding">
          <Route index element={<Onboarding />} />
          <Route path="steps/:stepId" element={<StepsLayout />} />
          <Route path="demo" element={<OnboardingDemo />} />
          <Route path="features" element={<FeaturesPage />} />
        </Route>

        <Route path="/" element={<Navigate to="/home" replace />} />
        <Route
          path="/personalize"
          element={<Navigate to="/home/personalize" replace />}
        />
        <Route
          path="/settings/connect-mcp"
          element={<Navigate to="/connect-apps" replace />}
        />
        <Route path="/audit" element={<Navigate to="/home" replace />} />
        <Route
          path="/observability"
          element={<Navigate to="/home" replace />}
        />
        <Route path="/executions" element={<Navigate to="/home" replace />} />
        <Route
          path="/agents"
          element={<Navigate to="/settings/ai" replace />}
        />
        <Route path="/agents/:agentId" element={<LegacyAgentRedirect />} />
        <Route path="/options/*" element={<OptionsRedirect />} />

        <Route path="*" element={<Navigate to="/home" replace />} />
      </Routes>
    </HashRouter>
  )
}
