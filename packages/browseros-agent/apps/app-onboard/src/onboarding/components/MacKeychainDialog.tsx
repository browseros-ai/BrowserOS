import { Dialog } from '@base-ui/react/dialog'
import { ArrowRight, Lock, X } from 'lucide-react'
import { type RefObject, useRef } from 'react'
import { Button } from '@/components/ui/button'
import {
  MacKeychainPermissionNote,
  MacKeychainPreview,
} from './MacKeychainNotice'

interface MacKeychainDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onContinue: () => void
  returnFocus: RefObject<HTMLButtonElement | null>
}

/** Prepares the user before Chromium requests Keychain access, without handling credentials. */
export function MacKeychainDialog({
  open,
  onOpenChange,
  onContinue,
  returnFocus,
}: MacKeychainDialogProps) {
  const titleRef = useRef<HTMLHeadingElement>(null)
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Backdrop className="fixed inset-0 z-40 bg-ink/25 backdrop-blur-[3px]" />
        <Dialog.Popup
          initialFocus={titleRef}
          finalFocus={returnFocus}
          className="fixed top-1/2 left-1/2 z-50 flex max-h-[calc(100dvh-32px)] w-[600px] max-w-[calc(100vw-32px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-5 overflow-y-auto rounded-[20px] border border-border-2 bg-card p-5 text-ink shadow-[0_24px_70px_#16244a26] outline-none sm:p-7"
        >
          <div className="flex items-center justify-between">
            <span className="flex size-10 items-center justify-center rounded-[13px] bg-accent-tint text-accent">
              <Lock aria-hidden className="size-6" />
            </span>
            <Dialog.Close
              render={
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label="Back to import options"
                />
              }
            >
              <X className="size-[18px]" />
            </Dialog.Close>
          </div>
          <div>
            <Dialog.Title
              ref={titleRef}
              tabIndex={-1}
              className="mb-3 font-semibold text-[26px] leading-[34px] tracking-tight outline-none"
            >
              Import your Chrome logins
            </Dialog.Title>
            <Dialog.Description className="text-[15px] text-ink-2 leading-[23px]">
              BrowserOS needs your permission to import your logins from Chrome.
              <br />
              When macOS asks, enter your Mac login password and click{' '}
              <strong className="font-semibold text-ink">Allow</strong>.
            </Dialog.Description>
          </div>
          <MacKeychainPreview />
          <MacKeychainPermissionNote />
          <div className="flex justify-end gap-2.5 pt-1">
            <Dialog.Close
              render={
                <Button
                  size="lg"
                  variant="outline"
                  className="h-[46px] rounded-lg px-5"
                />
              }
            >
              Back
            </Dialog.Close>
            <Button
              type="button"
              size="lg"
              className="h-[46px] rounded-lg px-5"
              onClick={onContinue}
            >
              Continue <ArrowRight aria-hidden className="size-4" />
            </Button>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
