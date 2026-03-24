import { Check, Copy, ExternalLink } from 'lucide-react'
import { type FC, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { PendingDeviceCode } from '@/lib/llm-providers/useOAuthProviderFlow'

interface DeviceCodeDialogProps {
  deviceCode: PendingDeviceCode | null
  onClose: () => void
}

export const DeviceCodeDialog: FC<DeviceCodeDialogProps> = ({
  deviceCode,
  onClose,
}) => {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    if (!deviceCode) return
    try {
      await navigator.clipboard.writeText(deviceCode.userCode)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Clipboard API failed
    }
  }

  return (
    <Dialog open={!!deviceCode} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Connect to {deviceCode?.providerName}</DialogTitle>
          <DialogDescription>
            Paste this code on the {deviceCode?.providerName} page that just
            opened in your browser.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col items-center gap-4 py-4">
          <div className="flex items-center gap-3 rounded-xl border-2 border-[var(--accent-orange)]/40 border-dashed bg-[var(--accent-orange)]/5 px-6 py-4">
            <code className="font-bold font-mono text-2xl text-foreground tracking-widest">
              {deviceCode?.userCode}
            </code>
            <Button
              variant="ghost"
              size="icon"
              onClick={handleCopy}
              className="shrink-0"
            >
              {copied ? (
                <Check className="h-4 w-4 text-green-600" />
              ) : (
                <Copy className="h-4 w-4" />
              )}
            </Button>
          </div>
          <p className="text-center text-muted-foreground text-xs">
            This dialog will close automatically once authentication completes.
          </p>
          {deviceCode?.verificationUri && (
            <a
              href={deviceCode.verificationUri}
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-1 text-[var(--accent-orange)] text-xs transition-colors hover:underline"
            >
              Open verification page
              <ExternalLink className="h-3 w-3" />
            </a>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
