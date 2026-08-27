import { Check, Loader2, MoreHorizontal } from 'lucide-react'
import type { FC, ReactNode } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { cn } from '@/lib/utils'

export interface TargetRowAction {
  label: string
  onSelect: () => void
  destructive?: boolean
  disabled?: boolean
}

export interface ConfiguredTargetRowProps {
  id: string
  name: string
  description: ReactNode
  icon: ReactNode
  /** Short noun for what kind of target this is: hosted, agent, api, local. */
  kind: string
  isSelected: boolean
  actions: TargetRowAction[]
  busy?: boolean
}

/**
 * One row in the providers list, shared by LLM providers and coding agents so
 * the two read as one set of things you can pick from.
 *
 * The default marker and the kind badge sit in fixed-width slots. Without
 * them, promoting a row adds a DEFAULT badge and drops the "Set as default"
 * button, which resizes the row and shifts every other row's badges sideways.
 *
 * Those slots reserve 220px, which is more than a narrow pane can spare, so
 * below `sm` the kind badge and the set-default button drop out and the name
 * takes the width back. Nothing is lost: the description already names the
 * adapter, and the row itself is the control that sets the default, so the
 * button was only ever a hover affordance for pointer users.
 */
export const ConfiguredTargetRow: FC<ConfiguredTargetRowProps> = ({
  id,
  name,
  description,
  icon,
  kind,
  isSelected,
  actions,
  busy = false,
}) => {
  const inputId = `target-${id}`

  return (
    <div
      className={cn(
        'group flex items-center gap-3 px-4 py-3 transition-colors',
        isSelected
          ? 'bg-[var(--accent-orange)]/5'
          : 'focus-within:bg-muted/40 hover:bg-muted/40',
      )}
    >
      <label
        htmlFor={inputId}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-3"
      >
        <input
          type="radio"
          id={inputId}
          name="default-provider"
          className="sr-only"
          checked={isSelected}
          onChange={() => {
            if (!isSelected) actions[0]?.onSelect()
          }}
        />
        <span
          className={cn(
            'flex size-5 shrink-0 items-center justify-center rounded-full border-2 transition-colors',
            isSelected
              ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]'
              : 'border-border group-hover:border-[var(--accent-orange)]/60',
          )}
        >
          {isSelected ? <Check className="size-3 text-white" /> : null}
        </span>
        <span className="flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]">
          {icon}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium text-sm">{name}</span>
          <span className="block truncate text-muted-foreground text-xs">
            {description}
          </span>
        </span>
      </label>

      <span className="hidden w-[72px] shrink-0 justify-start sm:flex">
        <Badge
          variant="secondary"
          className="rounded font-semibold text-[10px] uppercase tracking-wide"
        >
          {kind}
        </Badge>
      </span>

      <span className="flex shrink-0 justify-end sm:w-[116px]">
        {isSelected ? (
          <Badge
            variant="secondary"
            className="rounded bg-[var(--accent-orange)]/15 font-bold text-[10px] text-[var(--accent-orange)] uppercase tracking-wide"
          >
            Default
          </Badge>
        ) : (
          <Button
            variant="outline"
            size="sm"
            className="hidden opacity-0 transition-opacity focus-visible:opacity-100 group-focus-within:opacity-100 group-hover:opacity-100 sm:inline-flex"
            onClick={() => actions[0]?.onSelect()}
          >
            Set as default
          </Button>
        )}
      </span>

      <span className="flex w-8 shrink-0 justify-end">
        {actions.length > 1 ? (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={`Actions for ${name}`}
                className="text-muted-foreground"
              >
                {busy ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <MoreHorizontal className="size-4" />
                )}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {actions.slice(1).map((action) => (
                <DropdownMenuItem
                  key={action.label}
                  disabled={action.disabled}
                  onClick={action.onSelect}
                  className={cn(
                    action.destructive &&
                      'text-destructive focus:text-destructive',
                  )}
                >
                  {action.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </span>
    </div>
  )
}
