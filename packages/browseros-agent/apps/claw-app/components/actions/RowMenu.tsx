import { useState } from 'react'
import {
  type OverflowActionItem,
  OverflowActions,
} from '@/components/motion/overflow-actions'
import { cn } from '@/lib/utils'

interface RowMenuProps {
  /** Names the row in the button's label, as in "More for Codex". */
  label: string
  actions: OverflowActionItem[]
}

/**
 * A row's extra actions behind one button. Closed, it is a plain icon button
 * like the prototype's row menu; the bordered pill only shows once it opens,
 * because a ring around a closed button reads as a second circle on hover.
 */
export function RowMenu({ label, actions }: RowMenuProps) {
  const [open, setOpen] = useState(false)

  return (
    <OverflowActions
      size="sm"
      expanded={open}
      onExpandedChange={setOpen}
      openLabel={`More for ${label}`}
      closeLabel="Close"
      primaryActions={[]}
      overflowActions={actions}
      collapseOnAction
      classNames={{
        root: 'shrink-0',
        track: cn(
          'transition-colors',
          open
            ? 'border-border-strong'
            : 'border-transparent bg-transparent p-0',
        ),
        toggle: cn(
          'bg-transparent text-ink-2 hover:bg-bg-sunken',
          !open && 'h-9 w-9',
        ),
        action: 'font-semibold text-ink-2',
      }}
    />
  )
}
