import { Check, Plus } from 'lucide-react'
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
import type { Provider } from './chatComponentTypes'

interface ChatProviderSelectorProps {
  providers: Provider[]
  selectedProvider: Provider
  onSelectProvider: (provider: Provider) => void
}

export const ChatProviderSelector: FC<
  PropsWithChildren<ChatProviderSelectorProps>
> = ({ children, providers, selectedProvider, onSelectProvider }) => {
  const [open, setOpen] = useState(false)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent side="bottom" align="start" className="w-48 p-0">
        <Command>
          <CommandInput placeholder="Search providers..." className="h-9" />
          <CommandList>
            <div className="my-2 px-2 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
              AI Provider
            </div>
            <CommandEmpty>No provider found</CommandEmpty>
            <CommandGroup>
              {providers.map((provider) => {
                const isSelected = selectedProvider.id === provider.id
                return (
                  <CommandItem
                    key={provider.id}
                    value={`${provider.id} ${provider.name}`}
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
                      {provider.type === 'browseros' ? (
                        <BrowserOSIcon size={22} />
                      ) : (
                        <ProviderIcon
                          type={provider.type as ProviderType}
                          size={18}
                        />
                      )}
                    </span>
                    <span className="flex-1 text-left text-sm">
                      {provider.name}
                    </span>
                    {isSelected && (
                      <Check className="h-3.5 w-3.5 text-[var(--accent-orange)]" />
                    )}
                  </CommandItem>
                )
              })}
            </CommandGroup>
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
