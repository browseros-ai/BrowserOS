import { type FC, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { Capabilities, Feature } from '@/lib/browseros/capabilities'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'
export type ToolbarSettingsState = {
  showLlmChat: boolean
  showToolbarLabels: boolean
  verticalTabsEnabled: boolean
  supportsVerticalTabs: boolean
}

const DEFAULT_NATIVE_TOOLBAR_SETTINGS_STATE: ToolbarSettingsState = {
  showLlmChat: true,
  showToolbarLabels: true,
  verticalTabsEnabled: true,
  supportsVerticalTabs: false,
}

export async function loadToolbarSettingsState(): Promise<ToolbarSettingsState> {
  try {
    const adapter = getBrowserOSAdapter()
    const [chatPref, labelsPref] = await Promise.all([
      adapter.getPref(BROWSEROS_PREFS.SHOW_LLM_CHAT),
      adapter.getPref(BROWSEROS_PREFS.SHOW_TOOLBAR_LABELS),
    ])
    const supportsVerticalTabs = await Capabilities.supports(
      Feature.VERTICAL_TABS_SUPPORT,
    )
    const verticalTabsEnabled = supportsVerticalTabs
      ? (await adapter.getPref(BROWSEROS_PREFS.VERTICAL_TABS_ENABLED))
          ?.value !== false
      : true

    return {
      showLlmChat: chatPref?.value !== false,
      showToolbarLabels: labelsPref?.value !== false,
      verticalTabsEnabled,
      supportsVerticalTabs,
    }
  } catch {
    return DEFAULT_NATIVE_TOOLBAR_SETTINGS_STATE
  }
}

export const ToolbarSettingsCard: FC = () => {
  const [showLlmChat, setShowLlmChat] = useState(true)
  const [showToolbarLabels, setShowToolbarLabels] = useState(true)
  const [verticalTabsEnabled, setVerticalTabsEnabled] = useState(true)
  const [supportsVerticalTabs, setSupportsVerticalTabs] = useState(false)
  const [isLoading, setIsLoading] = useState(true)

  useEffect(() => {
    const loadPrefs = async () => {
      try {
        const state = await loadToolbarSettingsState()
        setShowLlmChat(state.showLlmChat)
        setShowToolbarLabels(state.showToolbarLabels)
        setVerticalTabsEnabled(state.verticalTabsEnabled)
        setSupportsVerticalTabs(state.supportsVerticalTabs)
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
      return true
    } catch {
      toast.error('Failed to update setting')
      return false
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

        {supportsVerticalTabs && (
          <div className="flex items-center justify-between border-border border-t pt-4">
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
