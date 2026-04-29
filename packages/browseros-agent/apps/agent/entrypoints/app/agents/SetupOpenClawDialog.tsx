import { Loader2 } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { ProviderOption } from './agents-page-types'
import { ProviderSelector } from './OpenClawControls'
import type { OpenClawCliProvider } from './openclaw-cli-providers'

interface SetupOpenClawDialogProps {
  defaultProviderId: string
  open: boolean
  providers: ProviderOption[]
  selectedProviderId: string
  selectedCliProvider: OpenClawCliProvider | undefined
  settingUp: boolean
  onOpenChange: (open: boolean) => void
  onProviderChange: (providerId: string) => void
  onSetup: () => void
}

export const SetupOpenClawDialog: FC<SetupOpenClawDialogProps> = ({
  defaultProviderId,
  open,
  providers,
  selectedProviderId,
  selectedCliProvider,
  settingUp,
  onOpenChange,
  onProviderChange,
  onSetup,
}) => (
  <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>Set Up OpenClaw</DialogTitle>
      </DialogHeader>
      <div className="space-y-4 py-2">
        <ProviderSelector
          providers={providers}
          defaultProviderId={defaultProviderId}
          selectedId={selectedProviderId}
          onSelect={onProviderChange}
          hideApiKeyHint={!!selectedCliProvider}
        />

        {selectedCliProvider ? (
          <p className="rounded-md border border-border bg-muted/30 px-3 py-2 text-muted-foreground text-xs">
            {selectedCliProvider.description}. Clicking{' '}
            <span className="font-medium">Set Up &amp; Start</span> starts the
            gateway and opens a terminal to sign in.
          </p>
        ) : null}

        <Button
          onClick={onSetup}
          disabled={settingUp || providers.length === 0}
          className="w-full"
        >
          {settingUp ? (
            <>
              <Loader2 className="mr-2 size-4 animate-spin" />
              Setting up...
            </>
          ) : (
            'Set Up & Start'
          )}
        </Button>
      </div>
    </DialogContent>
  </Dialog>
)
