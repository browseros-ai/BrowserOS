import { Globe, X } from 'lucide-react'
import type { FC } from 'react'

export interface ChatAttachedTabsProps {
  tabs: chrome.tabs.Tab[]
  onRemoveTab: (tabId?: number) => void
}

export const ChatAttachedTabs: FC<ChatAttachedTabsProps> = ({
  tabs,
  onRemoveTab,
}) => {
  if (tabs.length === 0) return null

  return (
    <div className="px-3 pt-2">
      <div className="styled-scrollbar flex items-center gap-2 overflow-x-auto pb-1">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            className="flex min-w-0 max-w-[200px] flex-shrink-0 items-center gap-1.5 rounded-lg border border-border bg-accent/50 px-2 py-1.5"
          >
            <div className="flex h-4 w-4 flex-shrink-0 items-center justify-center rounded border border-border bg-background">
              {tab.favIconUrl ? (
                <img src={tab.favIconUrl} alt="" className="h-3 w-3" />
              ) : (
                <Globe className="h-3 w-3 text-muted-foreground" />
              )}
            </div>
            <div className="flex-1 truncate font-medium text-foreground text-xs">
              {tab.title}
            </div>
            <button
              type="button"
              onClick={() => onRemoveTab(tab.id)}
              className="flex-shrink-0 rounded p-0.5 transition-colors hover:bg-background"
              title="Remove tab"
            >
              <X className="h-3 w-3 text-muted-foreground" />
            </button>
          </div>
        ))}
      </div>
    </div>
  )
}
