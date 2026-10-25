import { ArrowRight, ArrowUpRight } from 'lucide-react'
import { useNavigate } from 'react-router'
import { Button } from '@/components/motion/button/base'
import { SettingRow, SettingsSection } from './settings.components'

export const GUIDE_URL = 'https://docs.browseros.com/browserclaw'
export const SUPPORT_DETAILS_ROUTE = '/diagnostics'

/**
 * Opens a page in a new browser tab through the tabs API, which the extension
 * holds and which also opens targets that a plain link would be blocked from.
 */
export function openInNewTab(url: string): void {
  chrome.tabs.create({ url })
}

/** The guide, and the details support asks for when something isn't working. */
export function HelpSettings() {
  const navigate = useNavigate()
  return (
    <SettingsSection title="Help">
      <SettingRow
        id="guide"
        title="Read the guide"
        description="Step-by-step help for setting up and using neo."
        control={
          <Button
            variant="secondary"
            size="sm"
            onClick={() => openInNewTab(GUIDE_URL)}
            className="font-semibold"
          >
            Open guide
            <ArrowUpRight aria-hidden className="size-4" />
          </Button>
        }
      />
      <SettingRow
        id="support-details"
        title="Details for support"
        description="Version and system details to share when something isn’t working."
        control={
          <Button
            variant="secondary"
            size="sm"
            onClick={() => navigate(SUPPORT_DETAILS_ROUTE)}
            className="font-semibold"
          >
            View details
            <ArrowRight aria-hidden className="size-4" />
          </Button>
        }
      />
    </SettingsSection>
  )
}
