import { useId } from 'react'
import { Checkbox } from '@/components/ui/checkbox'
import type { BrowserOSImportItem } from '../browseros-onboarding-api'
import { importItemLabel } from '../onboarding-v2.helpers'

interface ImportItemChecklistProps {
  items: readonly BrowserOSImportItem[]
  checkedItems: readonly BrowserOSImportItem[]
  onToggle: (item: BrowserOSImportItem) => void
}

/** Renders the per-item import selector for the active browser source. */
export function ImportItemChecklist({
  items,
  checkedItems,
  onToggle,
}: ImportItemChecklistProps) {
  const checklistId = useId()
  const checkedItemSet = new Set(checkedItems)
  return (
    <div className="mb-4 rounded-xl border border-border-2 bg-card p-4">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div className="font-bold text-[12.5px] text-ink-2">What to import</div>
        <div className="font-mono text-[11.5px] text-ink-3">
          {checkedItems.length} of {items.length} selected
        </div>
      </div>
      <div className="grid grid-cols-2 gap-x-4 gap-y-2.5">
        {items.map((item) => {
          const checked = checkedItemSet.has(item)
          const controlId = `${checklistId}-${item}`
          return (
            <label
              key={item}
              htmlFor={controlId}
              className="flex cursor-pointer items-center gap-2.5"
            >
              <Checkbox
                id={controlId}
                checked={checked}
                onCheckedChange={() => onToggle(item)}
              />
              <span className="text-[12.5px] text-ink">
                {importItemLabel(item)}
              </span>
            </label>
          )
        })}
      </div>
    </div>
  )
}
