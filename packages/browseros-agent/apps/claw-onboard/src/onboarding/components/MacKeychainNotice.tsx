import { Lock } from 'lucide-react'

/** Explains the macOS Keychain prompt shown during Chrome data import. */
export function MacKeychainNotice() {
  return (
    <div className="mb-4 flex items-start gap-3 rounded-xl border border-blue/20 bg-accent-tint p-4">
      <Lock className="mt-0.5 size-[18px] shrink-0 text-blue" />
      <div className="text-[12.5px] text-ink-2 leading-[1.5]">
        <span className="font-semibold text-ink">
          macOS will ask permission
        </span>{' '}
        to read Chrome&rsquo;s saved data. That&rsquo;s expected. Click{' '}
        <span className="font-semibold text-ink">Allow</span> on the Keychain
        prompt.
      </div>
    </div>
  )
}
