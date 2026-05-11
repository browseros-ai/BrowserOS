import type { useCombobox } from 'downshift'
import { Search, Sparkles } from 'lucide-react'
import { motion } from 'motion/react'
import type { FC } from 'react'
import { cn } from '@/lib/utils'
import type { SuggestionItem, SuggestionSection } from './lib/suggestions/types'

type GetMenuProps = ReturnType<typeof useCombobox>['getMenuProps']
type GetItemProps = ReturnType<typeof useCombobox>['getItemProps']

interface SearchSuggestionsProps {
  getMenuProps: GetMenuProps
  getItemProps: GetItemProps
  sections: SuggestionSection[]
  highlightedIndex: number
}

const SectionTitle: FC<{ title: string }> = ({ title }) =>
  title ? (
    <div className="mb-2 px-2 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
      {title}
    </div>
  ) : null

const SuggestionItemRenderer: FC<{
  item: SuggestionItem
  isHighlighted: boolean
  getItemProps: GetItemProps
  index: number
}> = ({ item, isHighlighted, getItemProps, index }) => {
  const baseClassName = cn(
    'ph-mask flex w-full items-center gap-3 rounded-lg p-3 text-left text-foreground text-sm transition-colors hover:bg-accent cursor-pointer',
    isHighlighted && 'bg-accent',
  )

  switch (item.type) {
    case 'search':
      return (
        <li className={baseClassName} {...getItemProps({ item, index })}>
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate" title={item.query}>
            {item.query}
          </span>
        </li>
      )

    case 'ai-tab':
      return (
        <li className={baseClassName} {...getItemProps({ item, index })}>
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10 transition-colors group-hover:bg-[var(--accent-orange)]/20">
            <item.icon className="h-4 w-4 text-[var(--accent-orange)]" />
          </div>
          <div className="flex-1 text-left">
            <div className="font-medium text-foreground text-sm">
              {item.name}
            </div>
            {item.description && (
              <div className="text-muted-foreground text-xs">
                {item.description}
              </div>
            )}
          </div>
        </li>
      )

    case 'browseros':
      return (
        <li className={baseClassName} {...getItemProps({ item, index })}>
          <Sparkles className="h-4 w-4 shrink-0 text-muted-foreground" />
          <span className="flex-shrink-0 font-semibold">Ask Shimmy:</span>
          <span
            className="min-w-0 flex-1 truncate"
            title={item.message || 'Type a message...'}
          >
            {item.message || 'Type a message...'}
          </span>
        </li>
      )
  }
}

export const SearchSuggestions: FC<SearchSuggestionsProps> = ({
  getItemProps,
  getMenuProps,
  sections,
  highlightedIndex,
}) => {
  let globalIndex = 0

  return (
    <motion.ul
      {...getMenuProps()}
      className="styled-scrollbar flex max-h-60 flex-col gap-2 overflow-y-auto px-2"
      transition={{ duration: 0.2 }}
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: 'auto' }}
      exit={{ opacity: 0, height: 0 }}
    >
      {sections.map((section) => (
        <div key={section.id} className="px-3">
          <SectionTitle title={section.title} />
          {section.items.map((item) => {
            const currentIndex = globalIndex++
            return (
              <SuggestionItemRenderer
                key={item.id}
                item={item}
                isHighlighted={highlightedIndex === currentIndex}
                getItemProps={getItemProps}
                index={currentIndex}
              />
            )
          })}
        </div>
      ))}
    </motion.ul>
  )
}
