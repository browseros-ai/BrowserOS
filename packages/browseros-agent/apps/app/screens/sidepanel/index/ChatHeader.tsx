import {
  Bot,
  ChevronDown,
  Github,
  History,
  Plus,
  SettingsIcon,
} from 'lucide-react'
import type { FC } from 'react'
import { Link, useLocation, useNavigate } from 'react-router'
import { BRAND_MARKS } from '@/components/agents/agent-brand-marks'
import { ChatProviderSelector } from '@/components/chat/ChatProviderSelector'
import type { Provider } from '@/components/chat/chatComponentTypes'
import { CreditBadge } from '@/components/credits/CreditBadge'
import { ThemeToggle } from '@/components/elements/theme-toggle'
import { Feature } from '@/lib/browseros/capabilities'
import { productRepositoryUrl } from '@/lib/constants/productUrls'
import { BrowserOSIcon, ProviderIcon } from '@/lib/llm-providers/providerIcons'
import type { ProviderType } from '@/lib/llm-providers/types'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import { useCredits } from '@/modules/credits/credits.hooks'

const CreditsBadgeWrapper: FC = () => {
  const { supports } = useCapabilities()
  const { data } = useCredits()
  if (!supports(Feature.CREDITS_SUPPORT) || data === undefined) return null
  return (
    <CreditBadge
      credits={data.credits}
      onClick={() => window.open('/app.html#/settings/usage', '_blank')}
    />
  )
}

export interface ChatHeaderProps {
  selectedProvider: Provider
  providers: Provider[]
  onSelectProvider: (provider: Provider) => void
  onNewConversation: () => void
  hasMessages: boolean
  hideHistory?: boolean
}

export const ChatHeader: FC<ChatHeaderProps> = ({
  selectedProvider,
  providers,
  onSelectProvider,
  onNewConversation,
  hasMessages,
  hideHistory,
}) => {
  const location = useLocation()
  const navigate = useNavigate()
  const isHistoryPage = location.pathname === '/history'

  const handleNewConversationFromHistory = () => {
    onNewConversation()
    navigate('/')
  }

  return (
    <header className="flex items-center justify-between border-border/40 border-b bg-background/80 px-3 py-2.5 backdrop-blur-md">
      <div className="flex items-center gap-2">
        {/* Provider Selector */}
        <ChatProviderSelector
          providers={providers}
          selectedProvider={selectedProvider}
          onSelectProvider={onSelectProvider}
        >
          <button
            type="button"
            className="group relative inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-border px-2 py-1.5 text-foreground transition-colors hover:border-[var(--accent-orange)]/40 hover:bg-muted/50 data-[state=open]:border-[var(--accent-orange)]/50 data-[state=open]:bg-accent"
            title="Change AI Provider"
          >
            <HeaderProviderIcon provider={selectedProvider} />
            <span className="font-semibold text-base">
              {selectedProvider.name}
            </span>
            <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
          </button>
        </ChatProviderSelector>
        {selectedProvider.type === 'browseros' && <CreditsBadgeWrapper />}
      </div>

      <div className="flex items-center gap-1">
        {!isHistoryPage && hasMessages && (
          <button
            type="button"
            onClick={onNewConversation}
            className="cursor-pointer rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
            title="New conversation"
          >
            <Plus className="h-4 w-4" />
          </button>
        )}

        {!hideHistory &&
          (isHistoryPage ? (
            <button
              type="button"
              onClick={handleNewConversationFromHistory}
              className="cursor-pointer rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
              title="New conversation"
            >
              <Plus className="h-4 w-4" />
            </button>
          ) : (
            <Link
              to="/history"
              className="cursor-pointer rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
              title="Chat history"
            >
              <History className="h-4 w-4" />
            </Link>
          ))}

        <a
          href={productRepositoryUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="cursor-pointer rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
          title="Star on Github"
        >
          <Github className="h-4 w-4" />
        </a>

        <a
          href="/app.html#/settings"
          target="_blank"
          rel="noopener noreferrer"
          className="cursor-pointer rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
          title="Settings"
        >
          <SettingsIcon className="h-4 w-4" />
        </a>

        <ThemeToggle
          className="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
          iconClassName="h-4 w-4"
        />
      </div>
    </header>
  )
}

function HeaderProviderIcon({ provider }: { provider: Provider }) {
  if (provider.kind === 'acp') {
    const Mark = BRAND_MARKS[provider.brandKey ?? '']
    return Mark ? (
      <Mark className="h-[18px] w-[18px]" />
    ) : (
      <Bot className="h-[18px] w-[18px]" />
    )
  }
  if (provider.type === 'browseros') return <BrowserOSIcon size={18} />
  return <ProviderIcon type={provider.type as ProviderType} size={18} />
}
