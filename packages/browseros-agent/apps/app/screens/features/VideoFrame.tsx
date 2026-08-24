import type { FC, PropsWithChildren } from 'react'
import { cn } from '@/lib/utils'

export interface VideoFrameProps {
  className?: string
  title?: string
}

export const VideoFrame: FC<PropsWithChildren<VideoFrameProps>> = ({
  children,
  className,
  title,
}) => {
  return (
    <div className={cn('mx-auto max-w-4xl', className)}>
      <div className="relative overflow-hidden rounded-xl border border-border bg-muted shadow-2xl">
        <div className="flex aspect-video items-center justify-center overflow-hidden rounded-xl bg-gradient-to-br from-muted to-background">
          {children}
        </div>
        <div className="absolute top-0 right-0 left-0 flex h-10 items-center gap-2 border-border border-b bg-background/95 px-4">
          <div className="flex gap-2">
            <div className="h-3 w-3 rounded-full bg-red-500/80" />
            <div className="h-3 w-3 rounded-full bg-yellow-500/80" />
            <div className="h-3 w-3 rounded-full bg-green-500/80" />
          </div>
          <div className="flex-1 text-center font-mono text-muted-foreground text-xs">
            {title}
          </div>
        </div>
      </div>
    </div>
  )
}
