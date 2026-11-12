import { Link2 } from 'lucide-react'
import { CopyButton } from '@/components/copy/CopyButton'
import { Card } from '@/components/motion/card'
import { Skeleton } from '@/components/ui/skeleton'

interface McpAddressCardProps {
  /** Endpoint URL from the page; null while it resolves. */
  endpointUrl: string | null
}

/** neo's MCP address, for any agent that asks for a server URL. */
export function McpAddressCard({ endpointUrl }: McpAddressCardProps) {
  return (
    <Card
      size="compact"
      className="flex-row flex-wrap items-center gap-x-4 gap-y-3 border-border-2 bg-card px-[18px] py-3.5 shadow-card"
    >
      <span className="flex items-center gap-3">
        <span
          aria-hidden
          className="grid size-9 shrink-0 place-items-center rounded-full bg-accent-tint text-primary"
        >
          <Link2 className="size-[18px]" />
        </span>
        <span className="font-semibold text-[15px] text-cyanotype-ink">
          neo’s MCP address
        </span>
      </span>
      <span className="flex min-w-0 flex-1 basis-72 items-center gap-2">
        {endpointUrl ? (
          <>
            <code className="min-w-0 flex-1 truncate rounded-md border border-border-2 bg-bg-sunken px-3 py-2 font-mono text-[13.5px] text-ink">
              {endpointUrl}
            </code>
            <CopyButton
              text={endpointUrl}
              what="neo’s MCP address"
              variant="secondary"
              size="sm"
              className="h-[38px] shrink-0 font-semibold text-[13.5px]"
            />
          </>
        ) : (
          <Skeleton
            role="status"
            aria-label="Finding the address"
            className="h-[38px] flex-1 rounded-md"
          />
        )}
      </span>
    </Card>
  )
}
