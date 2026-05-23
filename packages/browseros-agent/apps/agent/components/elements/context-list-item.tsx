import { Bookmark, Brain, Check, FileText, Globe } from 'lucide-react'
import type { FC } from 'react'
import { cn } from '@/lib/utils'
import type { ContextItem } from './use-context-sources'

interface ContextListItemProps {
  item: ContextItem
  isSelected: boolean
  className?: string
}

export const ContextListItem: FC<ContextListItemProps> = ({
  item,
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
      <ContextIcon item={item} />
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium text-foreground text-xs">
          {item.title}
        </div>
        <div className="truncate text-[10px] text-muted-foreground">
          {item.subtitle || item.url}
        </div>
      </div>
      <div className="flex-shrink-0 px-1 text-[8px] font-bold uppercase text-muted-foreground/50 border border-border/30 rounded">
        {item.type}
      </div>
    </div>
  )
}

const ContextIcon: FC<{ item: ContextItem }> = ({ item }) => (
  <div className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded border border-border bg-background">
    {item.icon ? (
      <img src={item.icon} alt="" className="h-3.5 w-3.5" />
    ) : item.type === 'tab' ? (
      <Globe className="h-3.5 w-3.5 text-muted-foreground" />
    ) : item.type === 'bookmark' ? (
      <Bookmark className="h-3.5 w-3.5 text-muted-foreground" />
    ) : item.type === 'file' ? (
      <FileText className="h-3.5 w-3.5 text-muted-foreground" />
    ) : (
      <Brain className="h-3.5 w-3.5 text-muted-foreground" />
    )}
  </div>
)
