import type { FC } from 'react'
import { Switch } from '@/components/ui/switch'
import { Label } from '@/components/ui/label'
import { cn } from '@/lib/utils'

interface ShieldToggleProps {
  id: string
  label: string
  description?: string
  checked: boolean
  onCheckedChange: (checked: boolean) => void
  disabled?: boolean
  className?: string
}

export const ShieldToggle: FC<ShieldToggleProps> = ({
  id,
  label,
  description,
  checked,
  onCheckedChange,
  disabled = false,
  className,
}) => {
  return (
    <div className={cn('flex items-center justify-between gap-4 py-2', className)}>
      <div className="flex flex-col gap-0.5">
        <Label htmlFor={id} className="text-sm font-medium leading-none">
          {label}
        </Label>
        {description && (
          <p className="text-xs text-muted-foreground">{description}</p>
        )}
      </div>
      <Switch
        id={id}
        checked={checked}
        onCheckedChange={onCheckedChange}
        disabled={disabled}
        aria-label={label}
      />
    </div>
  )
}
