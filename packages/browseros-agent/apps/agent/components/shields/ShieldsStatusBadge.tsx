import type { FC } from 'react'
import { Shield, ShieldCheck, ShieldOff } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import type { ShieldsConfig } from '@jarvisos/privacy/shields/shields.types'
import { AdblockMode } from '@jarvisos/privacy/shields/shields.types'

interface ShieldsStatusBadgeProps {
  config: ShieldsConfig
  blockedCount?: number
  className?: string
}

export const ShieldsStatusBadge: FC<ShieldsStatusBadgeProps> = ({
  config,
  blockedCount = 0,
  className,
}) => {
  const isFullyEnabled = config.enabled && config.adblock !== AdblockMode.Disabled
  const isPartiallyEnabled = config.enabled && config.adblock === AdblockMode.Disabled

  const Icon = isFullyEnabled ? ShieldCheck : isPartiallyEnabled ? Shield : ShieldOff

  const label = isFullyEnabled
    ? `Shields ON · ${blockedCount} blocked`
    : isPartiallyEnabled
      ? 'Shields partial'
      : 'Shields OFF'

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge
          variant="outline"
          className={cn(
            'flex items-center gap-1.5 cursor-default select-none px-2 py-0.5 text-xs',
            isFullyEnabled && 'border-[var(--accent-orange)]/40 text-[var(--accent-orange)]',
            !config.enabled && 'opacity-50',
            className,
          )}
        >
          <Icon className="h-3 w-3" />
          {blockedCount > 0 && <span>{blockedCount}</span>}
        </Badge>
      </TooltipTrigger>
      <TooltipContent side="bottom" className="text-xs">
        {label}
      </TooltipContent>
    </Tooltip>
  )
}
