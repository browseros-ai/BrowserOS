import type { FC } from 'react'
import { useNavigate } from 'react-router'
import { commitChatTargetSelection } from '@/modules/chat/sidepanel-chat-targets'
import { useLlmProviders } from '@/modules/llm-providers/llm-providers.hooks'
import { AddProviderSection } from '@/screens/ai-settings/AddProviderSection'
import {
  AddProviderDialogs,
  useAddProvider,
} from '@/screens/ai-settings/add-provider.hooks'

/**
 * First-run setup, reached from the native onboarding rather than the sidebar.
 *
 * Deliberately not BrowserOsAiPane: this is someone's first minute with the
 * product, so it carries the catalogue and nothing else, no configured list,
 * promos, default-target control or delete flows. It renders outside every
 * layout route, so there is no sidebar either.
 *
 * The handoff to the new tab page is a direct callback from the add itself, not
 * a reaction to the provider/agent lists changing: every add path already
 * funnels through one success point, so there is nothing to watch or debounce.
 */
export const OnboardingAiPage: FC = () => {
  const navigate = useNavigate()
  const { providers, saveProvider, setDefaultProvider } = useLlmProviders()

  const goHome = () => navigate('/home', { replace: true })

  // Adding a provider or a coding agent both count as connecting something, so
  // either makes what was just added the active chat target and then hands off.
  // commitChatTargetSelection writes the unified selection new chats read (and
  // updates the default-provider id for an LLM target); await it before the hop
  // so the new tab page opens on the target the user just set up.
  const addProvider = useAddProvider({
    providers,
    saveProvider,
    onProviderAdded: async (provider) => {
      await commitChatTargetSelection(
        { kind: 'llm', id: provider.id },
        { setDefaultProvider },
      )
      goHome()
    },
    onAgentAdded: async (agentId) => {
      await commitChatTargetSelection(
        { kind: 'acp', id: agentId },
        { setDefaultProvider },
      )
      goHome()
    },
  })

  return (
    <div className="min-h-[100dvh] bg-background">
      <div className="mx-auto w-full max-w-2xl px-6 py-14">
        <h1 className="mb-1.5 font-semibold text-3xl tracking-tight">
          Set up your <span className="text-[var(--accent-orange)]">agent</span>
        </h1>
        <p className="mb-8 text-muted-foreground">
          BrowserOS runs on the provider you bring. Connect an LLM provider or a
          coding agent harness you already use. You can change this any time in
          settings.
        </p>

        <AddProviderSection
          onCreateAgent={addProvider.onCreateAgent}
          onCreateCustomAgent={addProvider.onCreateCustomAgent}
          onUseTemplate={addProvider.onUseTemplate}
        />

        <AddProviderDialogs controller={addProvider} />
      </div>
    </div>
  )
}
