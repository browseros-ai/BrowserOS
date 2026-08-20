import { ArrowRight } from 'lucide-react'
import { Button } from '@/components/ui/button'

export function App() {
  return (
    <main className="flex min-h-dvh items-center justify-center p-6">
      <div className="w-full max-w-md space-y-6 text-center">
        <span className="inline-flex items-center rounded-full border border-border bg-card px-3 py-1 font-medium text-muted-foreground text-xs">
          BrowserOS Onboarding
        </span>
        <div className="space-y-2">
          <h1 className="font-semibold text-3xl text-foreground tracking-tight">
            Welcome to BrowserOS
          </h1>
          <p className="text-muted-foreground text-sm">
            Standalone onboarding shell. App theme, base-ui components.
          </p>
        </div>
        <Button size="lg">
          Get started
          <ArrowRight />
        </Button>
      </div>
    </main>
  )
}
