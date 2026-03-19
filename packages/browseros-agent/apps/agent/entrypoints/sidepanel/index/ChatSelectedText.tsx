import { FileText, X } from 'lucide-react'
import type { FC } from 'react'
import type { SelectedTextData } from '@/lib/selected-text/selectedTextStorage'

const MAX_DISPLAY_LENGTH = 200

interface ChatSelectedTextProps {
  selectedText: SelectedTextData
  onDismiss: () => void
}

export const ChatSelectedText: FC<ChatSelectedTextProps> = ({
  selectedText,
  onDismiss,
}) => {
  const truncated =
    selectedText.text.length > MAX_DISPLAY_LENGTH
      ? `${selectedText.text.slice(0, MAX_DISPLAY_LENGTH)}...`
      : selectedText.text

  return (
    <div className="px-3 pt-2">
      <div className="relative rounded-lg border border-[var(--accent-orange)]/30 bg-accent/30">
        <div className="flex items-start gap-2 px-3 py-2">
          <FileText className="mt-0.5 h-3.5 w-3.5 flex-shrink-0 text-[var(--accent-orange)]" />
          <div className="min-w-0 flex-1">
            <div className="mb-0.5 truncate font-medium text-[10px] text-muted-foreground">
              {selectedText.pageTitle}
            </div>
            <div className="line-clamp-3 text-foreground text-xs leading-relaxed">
              &ldquo;{truncated}&rdquo;
            </div>
          </div>
          <button
            type="button"
            onClick={onDismiss}
            className="flex-shrink-0 rounded p-0.5 transition-colors hover:bg-background"
            title="Remove selected text"
          >
            <X className="h-3 w-3 text-muted-foreground" />
          </button>
        </div>
      </div>
    </div>
  )
}
