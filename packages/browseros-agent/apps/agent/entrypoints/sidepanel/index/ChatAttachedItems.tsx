import { Bookmark, Brain, FileText, Globe, X } from 'lucide-react'
import type { FC } from 'react'
import type { ContextItem } from '@/components/elements/use-context-sources'

interface ChatAttachedItemsProps {
  items: ContextItem[]
  onRemoveItem: (itemId: string) => void
}

/**
 * Component to display attached context items (tabs, bookmarks, etc.) in ChatFooter
 */
export const ChatAttachedItems: FC<ChatAttachedItemsProps> = ({
  items,
  onRemoveItem,
}) => {
  if (items.length === 0) return null

  return (
    <div className="px-3 pt-2">
      <div className="styled-scrollbar flex items-center gap-2 overflow-x-auto pb-1">
        {items.map((item) => (
          <div
            key={item.id}
            className="flex min-w-0 max-w-[200px] flex-shrink-0 items-center gap-1.5 rounded-lg border border-border bg-accent/50 px-2 py-1.5"
          >
            <div className="flex h-4 w-4 flex-shrink-0 items-center justify-center rounded border border-border bg-background">
              {item.icon ? (
                <img src={item.icon} alt="" className="h-3.5 w-3.5" />
              ) : item.type === 'tab' ? (
                <Globe className="h-3 w-3 text-muted-foreground" />
              ) : item.type === 'bookmark' ? (
                <Bookmark className="h-3 w-3 text-muted-foreground" />
              ) : item.type === 'file' ? (
                <FileText className="h-3 w-3 text-muted-foreground" />
              ) : (
                <Brain className="h-3 w-3 text-muted-foreground" />
              )}
            </div>
            <div className="flex-1 truncate font-medium text-foreground text-xs">
              {item.title}
            </div>
            <button
              type="button"
              onClick={() => onRemoveItem(item.id)}
              className="flex-shrink-0 rounded p-0.5 transition-colors hover:bg-background"
              title={`Remove ${item.type}`}
            >
              <X className="h-3 w-3 text-muted-foreground" />
            </button>
          </div>
        ))}
      </div>
    </div>
  )
}
