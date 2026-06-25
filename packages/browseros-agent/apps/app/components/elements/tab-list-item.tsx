import { Check, Globe } from 'lucide-react'
import type { FC } from 'react'
import { cn } from '@/lib/utils'

export interface TabListItemProps {
  tab: chrome.tabs.Tab
  isSelected: boolean
  className?: string
}

export const TabListItem: FC<TabListItemProps> = ({
  tab,
  isSelected,
  className,
}) => {
  return (
    <div
      className={cn(
        'flex w-full cursor-pointer items-center gap-3 rounded-lg p-2.5 transition-colors',
        className,
      )}
    >
      <div
        className={cn(
          'flex h-5 w-5 flex-shrink-0 items-center justify-center rounded border transition-colors',
          isSelected
            ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]'
            : 'border-border bg-background',
        )}
      >
        {isSelected && <Check className="h-3 w-3 text-white" />}
      </div>
      <TabFavicon url={tab.favIconUrl} />
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium text-foreground text-xs">
          {tab.title}
        </div>
        <div className="truncate text-[10px] text-muted-foreground">
          {tab.url}
        </div>
      </div>
    </div>
  )
}

const TabFavicon: FC<{ url?: string }> = ({ url }) => (
  <div className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded border border-border bg-background">
    {url ? (
      <img src={url} alt="" className="h-3.5 w-3.5" />
    ) : (
      <Globe className="h-3.5 w-3.5 text-muted-foreground" />
    )}
  </div>
)
