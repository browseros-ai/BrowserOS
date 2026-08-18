import { ChevronRight } from 'lucide-react'
import { useId } from 'react'
import { Checkbox } from '@/components/ui/checkbox'
import type { BrowserOSImportItem } from '../browseros-onboarding-api'
import { importItemLabel, splitImportSelection } from '../onboarding-v2.helpers'

interface ImportItemChecklistProps {
  items: readonly BrowserOSImportItem[]
  checkedItems: readonly BrowserOSImportItem[]
  onToggle: (item: BrowserOSImportItem) => void
}

export function ImportItemChecklist({
  items,
  checkedItems,
  onToggle,
}: ImportItemChecklistProps) {
  const checklistId = useId()
  const checkedItemSet = new Set(checkedItems)
  const { defaultItems, optionalItems } = splitImportSelection(items)

  function renderRow(item: BrowserOSImportItem) {
    const controlId = `${checklistId}-${item}`
    return (
      <label
        key={item}
        htmlFor={controlId}
        className="flex cursor-pointer items-center gap-2.5"
      >
        <Checkbox
          id={controlId}
          checked={checkedItemSet.has(item)}
          onCheckedChange={() => onToggle(item)}
        />
        <span className="text-[12.5px] text-ink">{importItemLabel(item)}</span>
      </label>
    )
  }

  return (
    <div className="mb-4 rounded-xl border border-border-2 bg-card p-4">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div className="font-bold text-[12.5px] text-ink-2">What to copy</div>
        <div className="font-mono text-[11.5px] text-ink-3">
          {checkedItems.length} selected
        </div>
      </div>
      <div className="grid grid-cols-2 gap-x-4 gap-y-2.5">
        {defaultItems.map(renderRow)}
      </div>
      {optionalItems.length > 0 && (
        <details className="group mt-3 border-border-2 border-t pt-3">
          <summary className="flex cursor-pointer list-none items-center gap-1.5 text-[12px] text-ink-3 transition-colors hover:text-ink-2">
            <ChevronRight className="size-3.5 transition-transform group-open:rotate-90" />
            Also copy my browsing setup
            <span className="text-ink-4">
              ({optionalItems.map(importItemLabel).join(', ').toLowerCase()})
            </span>
          </summary>
          <div className="mt-2.5 grid grid-cols-2 gap-x-4 gap-y-2.5">
            {optionalItems.map(renderRow)}
          </div>
        </details>
      )}
    </div>
  )
}
