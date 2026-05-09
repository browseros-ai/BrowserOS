import { Brain, FileText, X } from 'lucide-react'
import type { FC } from 'react'
import type { ContextAttachment } from '@/lib/context-attachments'

interface ChatAttachedContextsProps {
  contexts: ContextAttachment[]
  onRemoveContext: (id: string) => void
}

export const ChatAttachedContexts: FC<ChatAttachedContextsProps> = ({
  contexts,
  onRemoveContext,
}) => {
  if (contexts.length === 0) return null

  return (
    <div className="px-3 pt-2">
      <div className="styled-scrollbar flex items-center gap-2 overflow-x-auto pb-1">
        {contexts.map((context) => {
          const Icon = context.kind === 'memory' ? Brain : FileText
          return (
            <div
              key={context.id}
              className="flex min-w-0 max-w-[220px] flex-shrink-0 items-center gap-1.5 rounded-lg border border-border bg-accent/50 px-2 py-1.5"
            >
              <div className="flex h-4 w-4 flex-shrink-0 items-center justify-center rounded border border-border bg-background">
                <Icon className="h-3 w-3 text-muted-foreground" />
              </div>
              <div className="flex-1 truncate font-medium text-foreground text-xs">
                {context.title}
              </div>
              <button
                type="button"
                onClick={() => onRemoveContext(context.id)}
                className="flex-shrink-0 rounded p-0.5 transition-colors hover:bg-background"
                title="Remove context"
              >
                <X className="h-3 w-3 text-muted-foreground" />
              </button>
            </div>
          )
        })}
      </div>
    </div>
  )
}
