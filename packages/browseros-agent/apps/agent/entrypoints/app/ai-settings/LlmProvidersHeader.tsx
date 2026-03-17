import { HelpCircle, Plus } from 'lucide-react'
import type { FC } from 'react'
import ProductLogoSvg from '@/assets/product_logo.svg'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { bringYourOwnLlmHelpUrl } from '@/lib/constants/productUrls'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'

interface LlmProvidersHeaderProps {
  providers: LlmProviderConfig[]
  defaultProviderId: string
  onDefaultProviderChange: (providerId: string) => void
  onAddProvider: () => void
}

/**
 * Header section for LLM providers with default provider selector and add button
 */
export const LlmProvidersHeader: FC<LlmProvidersHeaderProps> = ({
  providers,
  defaultProviderId,
  onDefaultProviderChange,
  onAddProvider,
}) => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <img src={ProductLogoSvg} alt="BrowserOS" className="h-8 w-8" />
        </div>
        <div className="flex-1">
          <div className="mb-1 flex items-center gap-2">
            <h2 className="font-semibold text-xl">LLM Providers</h2>
            <TooltipProvider delayDuration={0}>
              <Tooltip>
                <TooltipTrigger asChild>
                  <a
                    href={bringYourOwnLlmHelpUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="rounded-full p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                  >
                    <HelpCircle className="h-4 w-4" />
                  </a>
                </TooltipTrigger>
                <TooltipContent>Learn more about LLM providers</TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>
          <p className="mb-6 text-muted-foreground text-sm">
            Add your provider and choose the default LLM
          </p>

          <div className="flex flex-col items-start gap-4 sm:flex-row sm:items-center">
            <label
              htmlFor="provider-picker"
              className="whitespace-nowrap font-medium text-sm"
            >
              Default Provider:
            </label>
            <Select
              value={defaultProviderId}
              onValueChange={onDefaultProviderChange}
            >
              <SelectTrigger
                id="provider-picker"
                className="w-full flex-1 sm:max-w-xs"
              >
                <SelectValue placeholder="Select a provider" />
              </SelectTrigger>
              <SelectContent>
                {providers.map((provider) => (
                  <SelectItem key={provider.id} value={provider.id}>
                    {provider.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button
              variant="outline"
              onClick={onAddProvider}
              className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
            >
              <Plus className="h-4 w-4" />
              Add custom provider
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
