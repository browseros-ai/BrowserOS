import { type FC, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { Capabilities, Feature } from '@/lib/browseros/capabilities'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'
import { openBrowserOSHomeOnStartupStorage } from '@/lib/startup/startup-storage'

export const ToolbarSettingsCard: FC = () => {
  const [showLlmChat, setShowLlmChat] = useState(true)
  const [showLlmHub, setShowLlmHub] = useState(true)
  const [showToolbarLabels, setShowToolbarLabels] = useState(true)
  const [verticalTabsEnabled, setVerticalTabsEnabled] = useState(true)
  const [openHomeOnStartup, setOpenHomeOnStartup] = useState(true)
  const [supportsVerticalTabs, setSupportsVerticalTabs] = useState(false)
  const [isLoading, setIsLoading] = useState(true)

  useEffect(() => {
    const loadPrefs = async () => {
      try {
        const adapter = getBrowserOSAdapter()
        const [chatPref, hubPref, labelsPref] = await Promise.all([
          adapter.getPref(BROWSEROS_PREFS.SHOW_LLM_CHAT),
          adapter.getPref(BROWSEROS_PREFS.SHOW_LLM_HUB),
          adapter.getPref(BROWSEROS_PREFS.SHOW_TOOLBAR_LABELS),
        ])
        setShowLlmChat(chatPref?.value !== false)
        setShowLlmHub(hubPref?.value !== false)
        setShowToolbarLabels(labelsPref?.value !== false)
        setOpenHomeOnStartup(await openBrowserOSHomeOnStartupStorage.getValue())

        const hasVerticalTabsSupport = await Capabilities.supports(
          Feature.VERTICAL_TABS_SUPPORT,
        )
        setSupportsVerticalTabs(hasVerticalTabsSupport)

        if (hasVerticalTabsSupport) {
          const verticalTabsPref = await adapter.getPref(
            BROWSEROS_PREFS.VERTICAL_TABS_ENABLED,
          )
          setVerticalTabsEnabled(verticalTabsPref?.value !== false)
        }
      } catch {
        // API not available - use defaults
      } finally {
        setIsLoading(false)
      }
    }

    loadPrefs()
  }, [])

  const handleToggle = async (
    prefKey: string,
    value: boolean,
    setter: (v: boolean) => void,
  ) => {
    try {
      const adapter = getBrowserOSAdapter()
      const success = await adapter.setPref(prefKey, value)
      if (!success) {
        throw new Error('Failed to update setting')
      }
      setter(value)
    } catch {
      toast.error('Failed to update setting')
    }
  }

  const handleStartupToggle = async (value: boolean) => {
    try {
      await openBrowserOSHomeOnStartupStorage.setValue(value)
      setOpenHomeOnStartup(value)
    } catch {
      toast.error('Failed to update setting')
    }
  }

  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <h3 className="mb-4 font-semibold text-lg">Toolbar Settings</h3>

      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <Label htmlFor="show-llm-chat" className="font-medium text-sm">
              Show Chat Button
            </Label>
            <p className="text-muted-foreground text-xs">
              Display the Chat button in the browser toolbar
            </p>
          </div>
          <Switch
            id="show-llm-chat"
            checked={showLlmChat}
            onCheckedChange={(checked) =>
              handleToggle(
                BROWSEROS_PREFS.SHOW_LLM_CHAT,
                checked,
                setShowLlmChat,
              )
            }
            disabled={isLoading}
          />
        </div>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <Label htmlFor="show-llm-hub" className="font-medium text-sm">
              Show Hub Button
            </Label>
            <p className="text-muted-foreground text-xs">
              Display the Hub button in the browser toolbar
            </p>
          </div>
          <Switch
            id="show-llm-hub"
            checked={showLlmHub}
            onCheckedChange={(checked) =>
              handleToggle(BROWSEROS_PREFS.SHOW_LLM_HUB, checked, setShowLlmHub)
            }
            disabled={isLoading}
          />
        </div>

        <div className="flex items-center justify-between border-border border-t pt-4">
          <div className="space-y-0.5">
            <Label
              htmlFor="show-toolbar-labels"
              className="font-medium text-sm"
            >
              Show Button Labels
            </Label>
            <p className="text-muted-foreground text-xs">
              Display text labels next to toolbar button icons
            </p>
          </div>
          <Switch
            id="show-toolbar-labels"
            checked={showToolbarLabels}
            onCheckedChange={(checked) =>
              handleToggle(
                BROWSEROS_PREFS.SHOW_TOOLBAR_LABELS,
                checked,
                setShowToolbarLabels,
              )
            }
            disabled={isLoading}
          />
        </div>

        <div className="flex items-center justify-between border-border border-t pt-4">
          <div className="space-y-0.5">
            <Label
              htmlFor="open-home-on-startup"
              className="font-medium text-sm"
            >
              Open BrowserOS Home on Startup
            </Label>
            <p className="text-muted-foreground text-xs">
              Start with the BrowserOS interface instead of a plain new tab
            </p>
          </div>
          <Switch
            id="open-home-on-startup"
            checked={openHomeOnStartup}
            onCheckedChange={handleStartupToggle}
            disabled={isLoading}
          />
        </div>

        {supportsVerticalTabs && (
          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <Label
                htmlFor="vertical-tabs-enabled"
                className="font-medium text-sm"
              >
                Use Vertical Tabs
              </Label>
              <p className="text-muted-foreground text-xs">
                Turn off to switch back to the horizontal tab strip
              </p>
            </div>
            <Switch
              id="vertical-tabs-enabled"
              checked={verticalTabsEnabled}
              onCheckedChange={(checked) =>
                handleToggle(
                  BROWSEROS_PREFS.VERTICAL_TABS_ENABLED,
                  checked,
                  setVerticalTabsEnabled,
                )
              }
              disabled={isLoading}
            />
          </div>
        )}
      </div>
    </div>
  )
}
