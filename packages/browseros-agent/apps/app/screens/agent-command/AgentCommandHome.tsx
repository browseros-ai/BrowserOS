import type { FC } from 'react'
import { useNavigate } from 'react-router'
import { BrowserClawPromoBanner } from '@/components/promo/BrowserClawPromoBanner'
import { ProductHuntBanner } from '@/components/promo/ProductHuntBanner'
import { Feature } from '@/lib/browseros/capabilities'
import { createBrowserOSAction } from '@/lib/chat-actions/types'
import { openSidePanelWithSearch } from '@/lib/messaging/sidepanel/openSidepanelWithSearch'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import { stagePendingHomeMessage } from '@/modules/chat/pending-home-message'
import { useChatTargetSelection } from '@/modules/chat/use-chat-target-selection'
import { useActiveHint } from '@/screens/newtab/index/active-hint.hooks'
import { ImportDataHint } from '@/screens/newtab/index/ImportDataHint'
import { RecentSites } from '@/screens/newtab/index/RecentSites'
import { ScheduleResults } from '@/screens/newtab/index/ScheduleResults'
import { SignInHint } from '@/screens/newtab/index/SignInHint'
import {
  ConversationInput,
  type ConversationInputSendInput,
} from './ConversationInput'
import { resolveHomeLlmRoutingMode } from './home-compose.helpers'

export const AgentCommandHome: FC = () => {
  const navigate = useNavigate()
  const activeHint = useActiveHint()
  const { supports, isLoading: capabilitiesLoading } = useCapabilities()
  const supportsInlineChat = supports(Feature.NEWTAB_CHAT_SUPPORT)
  const llmRoutingMode = resolveHomeLlmRoutingMode({
    capabilitiesLoading,
    supportsInlineChat,
  })
  // Shared selection: the picker is derived from the persisted selection (kept in
  // sync with the sidebar and settings), so a third-party agent chosen here is
  // restored on load and falls back to the default LLM provider while agents load.
  const {
    chatTargets,
    providerOptions,
    selectedProvider,
    selectProvider,
    selectChatTarget,
  } = useChatTargetSelection()
  const waitingForLlmCapabilities =
    selectedProvider?.kind === 'llm' && llmRoutingMode === 'wait'

  const handleSend = async (input: ConversationInputSendInput) => {
    if (!selectedProvider) return
    if (selectedProvider.kind === 'llm' && llmRoutingMode === 'wait') return
    const target = chatTargets.find(
      (entry) =>
        entry.kind === selectedProvider.kind &&
        entry.id === selectedProvider.id,
    )
    if (!target) return
    await selectChatTarget(target)
    if (target.kind === 'llm' && llmRoutingMode === 'sidepanel') {
      const action = createBrowserOSAction({
        mode: 'chat',
        message: input.text,
        tabs: input.selectedTabs,
      })
      await openSidePanelWithSearch('open', {
        query: input.text,
        mode: 'chat',
        action,
      })
      return
    }
    const search = new URLSearchParams({ q: input.text, mode: 'agent' })
    if (input.attachments.length > 0) {
      search.set(
        'handoff',
        stagePendingHomeMessage({
          text: input.text,
          attachments: input.attachments,
        }),
      )
    }
    const tabIds = input.selectedTabs
      .map((tab) => tab.id)
      .filter((id): id is number => id !== undefined)
    if (tabIds.length > 0) search.set('tabs', tabIds.join(','))
    navigate(`/home/chat?${search.toString()}`)
  }

  return (
    <div className="min-h-full px-4 py-6">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-8">
        <div className="flex flex-col items-center gap-5 pt-[max(10vh,24px)] text-center">
          <div className="space-y-3">
            <h1 className="font-semibold text-[clamp(2.25rem,4.5vw,3.5rem)] leading-[1.08] tracking-[-0.025em] [text-wrap:balance]">
              What should your agent{' '}
              <span className="font-medium text-[var(--accent-orange)] italic">
                work on
              </span>{' '}
              next?
            </h1>
            <p className="mx-auto max-w-2xl text-muted-foreground text-sm leading-6 [text-wrap:pretty]">
              Pick BrowserOS AI or any agent, then start a task — all without
              leaving this tab.
            </p>
          </div>

          <div className="w-full max-w-3xl">
            <ConversationInput
              variant="home"
              providers={providerOptions}
              selectedProvider={selectedProvider}
              onSelectProvider={selectProvider}
              onSend={handleSend}
              streaming={false}
              disabled={!selectedProvider || waitingForLlmCapabilities}
              attachmentsEnabled={selectedProvider?.kind === 'acp'}
              placeholder={
                selectedProvider
                  ? `Ask ${selectedProvider.name} to handle a task...`
                  : 'Loading providers...'
              }
              onOpenVoiceMode={() => {
                navigate('/home/chat?voice=open&mode=chat')
              }}
            />
          </div>
        </div>

        <div className="mx-auto flex w-full max-w-3xl flex-col gap-10 pb-12">
          <RecentSites />
          <ProductHuntBanner fallback={<BrowserClawPromoBanner />} />
          <ScheduleResults />
        </div>
      </div>

      {activeHint === 'signin' ? <SignInHint /> : null}
      {activeHint === 'import' ? <ImportDataHint /> : null}
    </div>
  )
}
