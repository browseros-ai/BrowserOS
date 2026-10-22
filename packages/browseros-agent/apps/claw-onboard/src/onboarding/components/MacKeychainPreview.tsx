import { cn } from '@/lib/utils'

/** An inert drawing of the OS-owned prompt; credentials only ever go to macOS. */
export function MacKeychainPreview({ className }: { className?: string }) {
  return (
    <div className={cn('keychain-preview', className)} aria-hidden="true">
      <div className="keychain-preview-content">
        <div className="keychain-lock">
          <span />
        </div>
        <div className="min-w-0 flex-1">
          <p className="m-0 font-bold text-[13px] leading-[17px]">
            BrowserOS neo Helper wants to use your confidential information
            stored in &ldquo;Chrome Safe Storage&rdquo; in your keychain.
          </p>
          <p className="my-3 text-xs leading-[17px]">
            To allow this, enter the &ldquo;login&rdquo; keychain password.
          </p>
          <div className="flex items-center gap-2 text-xs">
            <span>Password:</span>
            <span className="keychain-password">••••••••</span>
          </div>
        </div>
      </div>
      <div className="keychain-preview-actions">
        <span className="keychain-help">?</span>
        <span className="keychain-button">Always Allow</span>
        <span className="flex-1" />
        <span className="keychain-button">Deny</span>
        <span className="keychain-allow">
          <span className="keychain-button">Allow</span>
        </span>
      </div>
    </div>
  )
}
