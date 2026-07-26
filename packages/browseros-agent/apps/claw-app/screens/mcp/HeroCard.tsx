import { EndpointStrip } from './EndpointStrip'

interface HeroCardProps {
  url: string | null
}

/** Renders the MCP hero around the resolved endpoint strip. */
export function HeroCard({ url }: HeroCardProps) {
  return (
    <header className="space-y-6">
      <div className="space-y-1">
        <h1 className="font-extrabold text-3xl leading-tight tracking-tight md:text-4xl">
          MCP
        </h1>
        <p className="text-[15px] text-ink-2 leading-snug">
          One endpoint,{' '}
          <span className="font-medium font-serif text-accent italic">
            every
          </span>{' '}
          harness.
        </p>
      </div>
      <EndpointStrip label="Endpoint URL" value={url} />
    </header>
  )
}
