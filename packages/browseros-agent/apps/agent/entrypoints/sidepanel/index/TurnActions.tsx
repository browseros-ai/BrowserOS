import { CopyIcon, PencilIcon, ReplyIcon } from 'lucide-react'
import { type FC, useState } from 'react'
import { MessageAction, MessageActions } from '@/components/ai-elements/message'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Textarea } from '@/components/ui/textarea'

export interface TurnActionsProps {
  /** The message ID (passed to onUndo/onFork callbacks) */
  messageId: string
  /** The message index in the array */
  messageIndex: number
  /** The user message text content (for edit prefill) */
  messageText: string
  /** Whether a stream is in progress */
  isStreaming: boolean
  /** Undo from this turn */
  onUndo: (messageId: string) => void
  /** Fork from this turn */
  onFork: (messageId: string) => void
  /** Edit this turn's user message and re-send */
  onEdit: (messageId: string, newText: string) => void
}

/**
 * Action buttons rendered on user messages: ↩ Undo, ⑂ Fork, ✏️ Edit.
 * Only visible when not streaming.
 */
export const TurnActions: FC<TurnActionsProps> = ({
  messageId,
  messageText,
  isStreaming,
  onUndo,
  onFork,
  onEdit,
}) => {
  const [editDialogOpen, setEditDialogOpen] = useState(false)
  const [editText, setEditText] = useState('')

  if (isStreaming) return null

  const handleEditClick = () => {
    setEditText(messageText)
    setEditDialogOpen(true)
  }

  const handleEditSubmit = () => {
    const trimmed = editText.trim()
    if (trimmed) {
      onEdit(messageId, trimmed)
    }
    setEditDialogOpen(false)
  }

  return (
    <>
      <MessageActions>
        <MessageAction
          onClick={() => onUndo(messageId)}
          label="Undo"
          tooltip="Undo from here"
        >
          <ReplyIcon className="size-3 rotate-180" />
        </MessageAction>
        <MessageAction
          onClick={() => onFork(messageId)}
          label="Fork"
          tooltip="Fork conversation from here"
        >
          <CopyIcon className="size-3" />
        </MessageAction>
        <MessageAction
          onClick={handleEditClick}
          label="Edit"
          tooltip="Edit and re-send"
        >
          <PencilIcon className="size-3" />
        </MessageAction>
      </MessageActions>

      <Dialog open={editDialogOpen} onOpenChange={setEditDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit message</DialogTitle>
          </DialogHeader>
          <Textarea
            value={editText}
            onChange={(e) => setEditText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                handleEditSubmit()
              }
            }}
            rows={3}
            autoFocus
          />
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={handleEditSubmit}>Send</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
