import { Bot, Check, Plus } from 'lucide-react'
import type { FC, PropsWithChildren } from 'react'
import { useState } from 'react'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { BrowserOSIcon, ProviderIcon } from '@/lib/llm-providers/providerIcons'
import type { ProviderType } from '@/lib/llm-providers/types'
import { cn } from '@/lib/utils'
import {
  getProviderSearchValue,
  getProviderSubtitle,
  groupProviderOptions,
} from './ChatProviderSelector.helpers'
import type { Provider } from './chatComponentTypes'

export interface ChatProviderSelectorProps {
  providers: Provider[]
  selectedProvider: Provider
  onSelectProvider: (provider: Provider) => void
}

export const ChatProviderSelector: FC<
  PropsWithChildren<ChatProviderSelectorProps>
> = ({ children, providers, selectedProvider, onSelectProvider }) => {
  const [open, setOpen] = useState(false)
  const groups = groupProviderOptions(providers)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent side="bottom" align="start" className="w-64 p-0">
        <Command>
          <CommandInput
            placeholder="Search providers or agents..."
            className="h-9"
          />
          <CommandList>
            <CommandEmpty>No provider found</CommandEmpty>
            {groups.map((group) => (
              <CommandGroup key={group.key} heading={group.label}>
                {group.options.map((provider) => {
                  const isSelected = selectedProvider.id === provider.id
                  const subtitle = getProviderSubtitle(provider)
                  return (
                    <CommandItem
                      key={provider.id}
                      value={getProviderSearchValue(provider, group.label)}
                      onSelect={() => {
                        onSelectProvider(provider)
                        setOpen(false)
                      }}
                      className={cn(
                        'flex w-full items-center gap-3 rounded-md p-2 transition-colors',
                        isSelected && 'bg-[var(--accent-orange)]/10',
                      )}
                    >
                      <span className="text-muted-foreground">
                        <ProviderOptionIcon provider={provider} />
                      </span>
                      <span className="min-w-0 flex-1 text-left">
                        <span className="block truncate text-sm">
                          {provider.name}
                        </span>
                        {subtitle && (
                          <span className="block truncate text-muted-foreground text-xs">
                            {subtitle}
                          </span>
                        )}
                      </span>
                      {isSelected && (
                        <Check className="h-3.5 w-3.5 text-[var(--accent-orange)]" />
                      )}
                    </CommandItem>
                  )
                })}
              </CommandGroup>
            ))}
            <div className="border-border border-t p-1">
              <button
                type="button"
                className="flex w-full items-center gap-3 rounded-md p-2 text-muted-foreground text-sm transition-colors hover:bg-accent hover:text-foreground"
                onClick={() => {
                  window.open('/app.html#/settings/ai', '_blank')
                  setOpen(false)
                }}
              >
                <Plus className="h-4 w-4" />
                Add Provider
              </button>
            </div>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

function ProviderOptionIcon({ provider }: { provider: Provider }) {
  if (provider.kind === 'acp') return <Bot size={18} />
  if (provider.type === 'browseros') return <BrowserOSIcon size={18} />
  return <ProviderIcon type={provider.type as ProviderType} size={18} />
}
