import type { FC } from 'react'
import { Shield, ShieldCheck, ShieldOff, ChevronDown } from 'lucide-react'
import { Switch } from '@/components/ui/switch'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Separator } from '@/components/ui/separator'
import { Badge } from '@/components/ui/badge'
import { ShieldToggle } from './ShieldToggle'
import { useJarvisShields } from '@/hooks/use-jarvis-shields'
import { FingerprintMode, CookieMode, AdblockMode, HttpsUpgradeMode } from '@jarvisos/privacy/shields/shields.types'
import { cn } from '@/lib/utils'

interface JarvisShieldsPanelProps {
  currentOrigin?: string
  className?: string
}

export const JarvisShieldsPanel: FC<JarvisShieldsPanelProps> = ({
  currentOrigin,
  className,
}) => {
  const { config, updateConfig, getSiteConfig, setSiteOverride } = useJarvisShields()
  const activeConfig = currentOrigin ? getSiteConfig(currentOrigin) : config

  const handleToggle = (key: keyof typeof config, value: boolean) => {
    if (currentOrigin) {
      setSiteOverride(currentOrigin, { [key]: value })
    } else {
      updateConfig({ [key]: value })
    }
  }

  const handleSelect = <K extends keyof typeof config>(key: K, value: typeof config[K]) => {
    if (currentOrigin) {
      setSiteOverride(currentOrigin, { [key]: value })
    } else {
      updateConfig({ [key]: value })
    }
  }

  const shieldIcon = activeConfig.enabled
    ? (activeConfig.adblock !== AdblockMode.Disabled ? ShieldCheck : Shield)
    : ShieldOff

  const ShieldIcon = shieldIcon

  return (
    <div className={cn('flex flex-col gap-1 rounded-xl border border-border bg-background p-4 shadow-sm min-w-[320px]', className)}>
      {/* Header */}
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-2">
          <ShieldIcon className={cn('h-5 w-5', activeConfig.enabled ? 'text-[var(--accent-orange)]' : 'text-muted-foreground')} />
          <span className="font-semibold text-sm">JarvisOS Shields</span>
        </div>
        <div className="flex items-center gap-2">
          {currentOrigin && (
            <Badge variant="outline" className="text-xs max-w-[120px] truncate">
              {currentOrigin}
            </Badge>
          )}
          <Switch
            checked={activeConfig.enabled}
            onCheckedChange={(v) => handleToggle('enabled', v)}
            aria-label="Toggle Shields"
          />
        </div>
      </div>

      <Separator className="my-1" />

      <div className={cn('flex flex-col gap-0.5 transition-opacity', !activeConfig.enabled && 'opacity-40 pointer-events-none')}>
        {/* Adblock */}
        <div className="flex items-center justify-between gap-4 py-2">
          <div className="flex flex-col gap-0.5">
            <span className="text-sm font-medium leading-none">Ad & Tracker Blocking</span>
            <span className="text-xs text-muted-foreground">Block ads and trackers</span>
          </div>
          <Select
            value={activeConfig.adblock}
            onValueChange={(v) => handleSelect('adblock', v as AdblockMode)}
          >
            <SelectTrigger className="w-[110px] h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={AdblockMode.Disabled}>Off</SelectItem>
              <SelectItem value={AdblockMode.Standard}>Standard</SelectItem>
              <SelectItem value={AdblockMode.Aggressive}>Aggressive</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {/* Fingerprint */}
        <div className="flex items-center justify-between gap-4 py-2">
          <div className="flex flex-col gap-0.5">
            <span className="text-sm font-medium leading-none">Fingerprint Protection</span>
            <span className="text-xs text-muted-foreground">Brave Farbling engine</span>
          </div>
          <Select
            value={activeConfig.fingerprintProtection}
            onValueChange={(v) => handleSelect('fingerprintProtection', v as FingerprintMode)}
          >
            <SelectTrigger className="w-[110px] h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={FingerprintMode.Disabled}>Off</SelectItem>
              <SelectItem value={FingerprintMode.Standard}>Standard</SelectItem>
              <SelectItem value={FingerprintMode.Strict}>Strict</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {/* Cookie */}
        <div className="flex items-center justify-between gap-4 py-2">
          <div className="flex flex-col gap-0.5">
            <span className="text-sm font-medium leading-none">Cookie Protection</span>
            <span className="text-xs text-muted-foreground">Block cross-site cookies</span>
          </div>
          <Select
            value={activeConfig.cookieProtection}
            onValueChange={(v) => handleSelect('cookieProtection', v as CookieMode)}
          >
            <SelectTrigger className="w-[110px] h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={CookieMode.Allow}>Allow All</SelectItem>
              <SelectItem value={CookieMode.CrossSiteBlocked}>Block Cross-site</SelectItem>
              <SelectItem value={CookieMode.AllBlocked}>Block All</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {/* HTTPS Upgrade */}
        <div className="flex items-center justify-between gap-4 py-2">
          <div className="flex flex-col gap-0.5">
            <span className="text-sm font-medium leading-none">HTTPS Upgrade</span>
            <span className="text-xs text-muted-foreground">Force secure connections</span>
          </div>
          <Select
            value={activeConfig.httpsUpgrade}
            onValueChange={(v) => handleSelect('httpsUpgrade', v as HttpsUpgradeMode)}
          >
            <SelectTrigger className="w-[110px] h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={HttpsUpgradeMode.Disabled}>Off</SelectItem>
              <SelectItem value={HttpsUpgradeMode.Standard}>Standard</SelectItem>
              <SelectItem value={HttpsUpgradeMode.Strict}>Strict</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <Separator className="my-1" />

        {/* Simple toggles */}
        <ShieldToggle
          id="referrer-trimming"
          label="Referrer Trimming"
          description="Trim referrer headers to protect origin"
          checked={activeConfig.referrerTrimming}
          onCheckedChange={(v) => handleToggle('referrerTrimming', v)}
        />
        <ShieldToggle
          id="bounce-tracking"
          label="Bounce Tracking Protection"
          description="Block bounce/redirect tracking"
          checked={activeConfig.bounceTrackingProtection}
          onCheckedChange={(v) => handleToggle('bounceTrackingProtection', v)}
        />
        <ShieldToggle
          id="tracker-block"
          label="Tracker Blocking"
          description="Block known tracking scripts"
          checked={activeConfig.trackerBlock}
          onCheckedChange={(v) => handleToggle('trackerBlock', v)}
        />
      </div>

      {/* Footer */}
      <Separator className="my-2" />
      <p className="text-xs text-muted-foreground text-center">
        Powered by Brave Shields + JarvisOS Privacy Layer
      </p>
    </div>
  )
}
