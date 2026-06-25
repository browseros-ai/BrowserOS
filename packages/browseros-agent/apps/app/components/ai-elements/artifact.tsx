'use client'

import { type LucideIcon, XIcon } from 'lucide-react'
import type { ComponentProps, HTMLAttributes } from 'react'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

export type ArtifactProps = HTMLAttributes<HTMLDivElement>

/** @public */
export const Artifact = ({ className, ...props }: ArtifactProps) => (
  <div
    className={cn(
      'flex flex-col overflow-hidden rounded-lg border bg-background shadow-sm',
      className,
    )}
    {...props}
  />
)

export type ArtifactHeaderProps = HTMLAttributes<HTMLDivElement>

/** @public */
export const ArtifactHeader = ({
  className,
  ...props
}: ArtifactHeaderProps) => (
  <div
    className={cn(
      'flex items-center justify-between border-b bg-muted/50 px-4 py-3',
      className,
    )}
    {...props}
  />
)

export type ArtifactCloseProps = ComponentProps<typeof Button>

/** @public */
export const ArtifactClose = ({
  className,
  children,
  size = 'sm',
  variant = 'ghost',
  ...props
}: ArtifactCloseProps) => (
  <Button
    className={cn(
      'size-8 p-0 text-muted-foreground hover:text-foreground',
      className,
    )}
    size={size}
    type="button"
    variant={variant}
    {...props}
  >
    {children ?? <XIcon className="size-4" />}
    <span className="sr-only">Close</span>
  </Button>
)

export type ArtifactTitleProps = HTMLAttributes<HTMLParagraphElement>

/** @public */
export const ArtifactTitle = ({ className, ...props }: ArtifactTitleProps) => (
  <p
    className={cn('font-medium text-foreground text-sm', className)}
    {...props}
  />
)

export type ArtifactDescriptionProps = HTMLAttributes<HTMLParagraphElement>

/** @public */
export const ArtifactDescription = ({
  className,
  ...props
}: ArtifactDescriptionProps) => (
  <p className={cn('text-muted-foreground text-sm', className)} {...props} />
)

export type ArtifactActionsProps = HTMLAttributes<HTMLDivElement>

/** @public */
export const ArtifactActions = ({
  className,
  ...props
}: ArtifactActionsProps) => (
  <div className={cn('flex items-center gap-1', className)} {...props} />
)

export type ArtifactActionProps = ComponentProps<typeof Button> & {
  tooltip?: string
  label?: string
  icon?: LucideIcon
}

/** @public */
export const ArtifactAction = ({
  tooltip,
  label,
  icon: Icon,
  children,
  className,
  size = 'sm',
  variant = 'ghost',
  ...props
}: ArtifactActionProps) => {
  const button = (
    <Button
      className={cn(
        'size-8 p-0 text-muted-foreground hover:text-foreground',
        className,
      )}
      size={size}
      type="button"
      variant={variant}
      {...props}
    >
      {Icon ? <Icon className="size-4" /> : children}
      <span className="sr-only">{label || tooltip}</span>
    </Button>
  )

  if (tooltip) {
    return (
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>{button}</TooltipTrigger>
          <TooltipContent>
            <p>{tooltip}</p>
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
    )
  }

  return button
}

export type ArtifactContentProps = HTMLAttributes<HTMLDivElement>

/** @public */
export const ArtifactContent = ({
  className,
  ...props
}: ArtifactContentProps) => (
  <div className={cn('flex-1 overflow-auto p-4', className)} {...props} />
)
