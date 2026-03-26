import { Coins } from 'lucide-react'
import type { FC } from 'react'
import { getCreditTextColor } from '@/lib/credits/credit-colors'
import { cn } from '@/lib/utils'

interface CreditBadgeProps {
  credits: number
  onClick?: () => void
}

export const CreditBadge: FC<CreditBadgeProps> = ({ credits, onClick }) => {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'inline-flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-0.5 font-medium text-xs transition-colors hover:bg-muted/50',
        getCreditTextColor(credits),
      )}
      title={`${credits} credits remaining`}
    >
      <Coins className="h-3.5 w-3.5" />
      <span>{credits}</span>
    </button>
  )
}
