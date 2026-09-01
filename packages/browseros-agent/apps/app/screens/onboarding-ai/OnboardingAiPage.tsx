import type { FC } from 'react'
import { useNavigate } from 'react-router'
import { useAcpAgents } from '@/modules/agents/agents.hooks'
import { useLlmProviders } from '@/modules/llm-providers/llm-providers.hooks'
import { AddProviderSection } from '@/screens/ai-settings/AddProviderSection'
import {
  AddProviderDialogs,
  useAddProvider,
} from '@/screens/ai-settings/add-provider.hooks'
import { useConnectionHandoff } from './onboarding-ai.hooks'

/**
 * First-run setup, reached from the native onboarding rather than the sidebar.
 *
 * Deliberately not BrowserOsAiPane: this is someone's first minute with the
 * product, so it carries the catalogue and nothing else, no configured list,
 * promos, default-target control or delete flows. It renders outside every
 * layout route, so there is no sidebar either.
 */
export const OnboardingAiPage: FC = () => {
  const navigate = useNavigate()
  const { providers, saveProvider, isLoading } = useLlmProviders()
  // `settled` rather than `loading`: the agents hook documents that `loading`
  // reads false for a render while the list is still empty, which would let
  // the handoff take its baseline before existing agents have arrived.
  const { agents, settled: agentsSettled } = useAcpAgents()
  const addProvider = useAddProvider({ providers, saveProvider })

  const goHome = () => navigate('/home', { replace: true })

  useConnectionHandoff({
    providers,
    agents,
    ready: !isLoading && agentsSettled,
    onConnected: goHome,
  })

  return (
    <div className="min-h-[100dvh] bg-background">
      <div className="mx-auto w-full max-w-2xl px-6 py-14">
        <h1 className="mb-1.5 font-semibold text-3xl tracking-tight">
          Set up your <span className="text-[var(--accent-orange)]">agent</span>
        </h1>
        <p className="mb-8 text-muted-foreground">
          Connect a provider or a coding agent harness you already use. You can
          change this any time in settings.
        </p>

        <AddProviderSection
          onCreateAgent={addProvider.onCreateAgent}
          onCreateCustomAgent={addProvider.onCreateCustomAgent}
          onUseTemplate={addProvider.onUseTemplate}
        />

        <div className="mt-10 border-border border-t pt-6">
          <button
            type="button"
            onClick={goHome}
            className="text-muted-foreground text-sm underline underline-offset-4 transition-colors hover:text-foreground"
          >
            Skip for now
          </button>
        </div>

        <AddProviderDialogs controller={addProvider} />
      </div>
    </div>
  )
}
