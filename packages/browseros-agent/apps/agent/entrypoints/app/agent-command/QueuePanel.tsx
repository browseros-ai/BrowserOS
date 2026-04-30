import { ListPlus, X } from 'lucide-react'
import type { FC } from 'react'
import {
  Queue,
  QueueItem,
  QueueItemAction,
  QueueItemActions,
  QueueItemAttachment,
  QueueItemContent,
  QueueItemFile,
  QueueItemImage,
  QueueList,
  QueueSection,
  QueueSectionContent,
  QueueSectionLabel,
  QueueSectionTrigger,
} from '@/components/ai-elements/queue'
import type {
  HarnessQueuedMessage,
  HarnessQueuedMessageAttachment,
} from '@/entrypoints/app/agents/agent-harness-types'
import { firstNonBlankLine } from '@/entrypoints/app/agents/agent-row/agent-row.helpers'

interface QueuePanelProps {
  queue: HarnessQueuedMessage[]
  onRemove: (messageId: string) => void
}

/**
 * Renders the agent's pending message queue using the shared AI
 * Elements `Queue` primitives. Caller is expected to gate render on
 * `queue.length > 0` — when empty, this returns null so the panel
 * disappears cleanly between turns.
 */
export const QueuePanel: FC<QueuePanelProps> = ({ queue, onRemove }) => {
  if (queue.length === 0) return null
  return (
    <Queue>
      <QueueSection>
        <QueueSectionTrigger>
          <QueueSectionLabel
            count={queue.length}
            label={queue.length === 1 ? 'queued message' : 'queued messages'}
            icon={<ListPlus className="size-3.5" />}
          />
        </QueueSectionTrigger>
        <QueueSectionContent>
          <QueueList>
            {queue.map((entry) => (
              <QueueItem key={entry.id}>
                <div className="flex items-center gap-2">
                  <QueueItemContent>
                    {firstNonBlankLine(entry.message)}
                  </QueueItemContent>
                  <QueueItemActions>
                    <QueueItemAction
                      aria-label="Remove from queue"
                      onClick={() => onRemove(entry.id)}
                    >
                      <X className="size-3" />
                    </QueueItemAction>
                  </QueueItemActions>
                </div>
                {entry.attachments && entry.attachments.length > 0 ? (
                  <QueueItemAttachment>
                    {entry.attachments.map((attachment, idx) =>
                      renderAttachment(entry.id, attachment, idx),
                    )}
                  </QueueItemAttachment>
                ) : null}
              </QueueItem>
            ))}
          </QueueList>
        </QueueSectionContent>
      </QueueSection>
    </Queue>
  )
}

function renderAttachment(
  messageId: string,
  attachment: HarnessQueuedMessageAttachment,
  idx: number,
) {
  if (attachment.mediaType.startsWith('image/')) {
    const src = `data:${attachment.mediaType};base64,${attachment.data}`
    return <QueueItemImage key={`${messageId}-${idx}`} src={src} />
  }
  return (
    <QueueItemFile key={`${messageId}-${idx}`}>
      {attachment.mediaType}
    </QueueItemFile>
  )
}
