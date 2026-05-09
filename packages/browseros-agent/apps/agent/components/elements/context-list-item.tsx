import { Brain, Check, FileText } from 'lucide-react'
import type { FC } from 'react'
import type { ContextAttachment } from '@/lib/context-attachments'
import { cn } from '@/lib/utils'

interface ContextListItemProps {
  attachment: ContextAttachment
  isSelected: boolean
  className?: string
}

export const ContextListItem: FC<ContextListItemProps> = ({
  attachment,
  isSelected,
  className,
}) => {
  const Icon = attachment.kind === 'memory' ? Brain : FileText

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
      <div className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded border border-border bg-background">
        <Icon className="h-3.5 w-3.5 text-muted-foreground" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium text-foreground text-xs">
          {attachment.title}
        </div>
        {attachment.source ? (
          <div className="truncate text-[10px] text-muted-foreground">
            {attachment.source}
          </div>
        ) : null}
      </div>
    </div>
  )
}
