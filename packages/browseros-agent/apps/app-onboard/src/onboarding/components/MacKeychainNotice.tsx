import { Info, Lock } from 'lucide-react'

/** An inert illustration of the OS-owned prompt; credentials only go to macOS. */
export function MacKeychainPreview() {
  return (
    <figure className="m-0 min-w-0">
      <figcaption className="mb-3 text-ink-2 text-xs">
        Example macOS dialog
      </figcaption>
      <div className="keychain-preview" aria-hidden="true">
        <div className="keychain-preview-content">
          <div className="keychain-lock">
            <span />
          </div>
          <div className="min-w-0 flex-1">
            <p className="m-0 font-bold text-[13px] leading-[17px]">
              BrowserOS Helper wants to use your confidential information stored
              in &ldquo;Chrome Safe Storage&rdquo; in your keychain.
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
    </figure>
  )
}

export function MacKeychainPermissionNote() {
  return (
    <p className="m-0 flex items-start gap-2 text-[13px] text-ink-2 leading-[19px]">
      <Info aria-hidden className="mt-0.5 size-4 shrink-0" />
      Allow gives access for this import.
    </p>
  )
}

/** Native progress does not expose whether a Keychain prompt is currently open. */
export function MacKeychainReminder() {
  return (
    <div className="mt-4 flex items-start gap-3 rounded-xl border border-accent/20 bg-accent-tint p-4 text-[13px] text-ink-2 leading-relaxed">
      <Lock aria-hidden className="mt-0.5 size-4 shrink-0 text-accent" />
      <p>
        If macOS asks, enter your Mac login password and click{' '}
        <strong className="font-semibold text-ink">Allow</strong>.
      </p>
    </div>
  )
}
