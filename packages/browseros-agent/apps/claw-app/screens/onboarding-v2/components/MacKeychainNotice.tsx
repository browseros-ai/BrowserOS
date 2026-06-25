import { Lock } from 'lucide-react'

/**
 * Blue info bar that explains the macOS Keychain permission prompt
 * the user is about to see. Phase 3 just renders it; the real prompt
 * lives in the wiring layer.
 */
export function MacKeychainNotice() {
  return (
    <div className="mb-4 flex items-start gap-3 rounded-xl border border-blue/20 bg-[#EEF3FA] p-4">
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
