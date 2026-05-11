'use client'

import type { FC } from 'react'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'

/**
 * Props for ActiveModelSelect
 * @public
 */
export interface ActiveModelSelectProps {
  /** All model IDs to show in the dropdown */
  models: string[]
  /** Currently active model ID */
  value: string
  /** Callback when selection changes */
  onChange: (modelId: string) => void
  /** Whether the select is disabled */
  disabled?: boolean
}

/**
 * A Select dropdown for choosing the active model from saved models.
 *
 * - Disabled when `models` is empty
 * - Falls back to placeholder when no value or empty list
 */
export const ActiveModelSelect: FC<ActiveModelSelectProps> = ({
  models,
  value,
  onChange,
  disabled = false,
}) => {
  const isDisabled = disabled || models.length === 0

  return (
    <Select value={value} onValueChange={onChange} disabled={isDisabled}>
      <SelectTrigger className={cn('w-full', isDisabled && 'opacity-50')}>
        <SelectValue
          placeholder={
            models.length === 0 ? 'No models available' : 'Select active model…'
          }
        />
      </SelectTrigger>
      <SelectContent>
        {models.map((modelId) => (
          <SelectItem key={modelId} value={modelId}>
            {modelId}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
