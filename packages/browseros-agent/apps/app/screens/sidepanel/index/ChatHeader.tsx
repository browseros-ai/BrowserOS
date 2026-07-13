import { Bot, ExternalLink, Github, History, MoreHorizontal, Plus, SettingsIcon } from 'lucide-react'
import type { FC } from 'react'
import { Link, useLocation, useNavigate } from 'react-router'
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
import { openDesktopSurface } from '@/lib/browseros/desktop-navigation'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'

const CreditsBadgeWrapper: FC = () => {
  const { supports } = useCapabilities()
  const { data } = useCredits()
  if (!supports(Feature.CREDITS_SUPPORT) || data === undefined) return null
  return (
    <CreditBadge
      credits={data.credits}
      onClick={() => openDesktopSurface('settings')}
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
  const providerLabel = selectedProvider.type.startsWith('opencode') || selectedProvider.name === 'OpenCode'
    ? 'Request Browser'
    : selectedProvider.name

  const handleNewConversationFromHistory = () => {
    onNewConversation()
    navigate('/')
  }

  const openRepository = () => {
    void chrome.tabs.create({ url: productRepositoryUrl })
  }

  const openSettings = () => {
    void chrome.runtime.openOptionsPage()
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
            className="group relative inline-flex min-w-0 max-w-[min(15rem,55vw)] cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground data-[state=open]:bg-accent"
            title={`Change AI provider. Current provider: ${selectedProvider.name}`}
            aria-label={`Change AI provider. Current provider: ${selectedProvider.name}`}
          >
            {selectedProvider.kind === 'acp' ? (
              <Bot className="h-[18px] w-[18px]" />
            ) : selectedProvider.type === 'browseros' ? (
              <BrowserOSIcon size={18} />
            ) : (
              <ProviderIcon
                type={selectedProvider.type as ProviderType}
                size={18}
              />
            )}
            <span className="truncate font-semibold text-base">
              {providerLabel}
            </span>
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

        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="cursor-pointer rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground data-[state=open]:bg-accent"
              title="More sidebar actions"
              aria-label="More sidebar actions"
            >
              <MoreHorizontal className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" sideOffset={8} className="min-w-48">
            <DropdownMenuLabel>Sidebar actions</DropdownMenuLabel>
            <DropdownMenuItem onSelect={openSettings}>
              <SettingsIcon className="h-4 w-4" />
              <span>Open settings</span>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={openRepository}>
              <Github className="h-4 w-4" />
              <span>Project on GitHub</span>
              <ExternalLink className="ml-auto h-3.5 w-3.5 text-muted-foreground" />
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={onNewConversation} disabled={!hasMessages}>
              <Plus className="h-4 w-4" />
              <span>New conversation</span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        <ThemeToggle
          className="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
          iconClassName="h-4 w-4"
        />
      </div>
    </header>
  )
}
