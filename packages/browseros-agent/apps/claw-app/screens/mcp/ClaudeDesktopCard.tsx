import { useState } from 'react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import { Button } from '@/components/motion/button/base'
import { AgentRow } from './ConnectionRow'
import { InstallExtensionDialog } from './InstallExtensionDialog'

/**
 * The Claude desktop app installs neo as an extension through its own
 * settings, which this app can neither do nor detect, so its row opens the
 * guided walkthrough instead of offering Connect.
 */
export function ClaudeDesktopCard() {
  const [guideOpen, setGuideOpen] = useState(false)

  return (
    <>
      <AgentRow
        className="bg-bg-sunken"
        logo={<HarnessIcon harness="Claude Code" className="size-[22px]" />}
        name="Claude desktop app"
        sub="Needs a one-time extension. Takes about a minute."
        action={
          <Button
            size="sm"
            onClick={() => setGuideOpen(true)}
            className="font-semibold"
          >
            Set up
          </Button>
        }
      />
      <InstallExtensionDialog open={guideOpen} onOpenChange={setGuideOpen} />
    </>
  )
}
