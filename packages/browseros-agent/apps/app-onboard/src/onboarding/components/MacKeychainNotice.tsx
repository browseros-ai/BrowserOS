import { Lock } from 'lucide-react'

/**
 * Explains the macOS Keychain prompt shown during Chrome data import, with a
 * shot of the real dialog so the button to press is unmistakable.
 *
 * Says Allow, not Always Allow: the importer reads the Keychain exactly once
 * per run (chrome_importer.cc extracts the Chrome Safe Storage key up front
 * and caches it for both cookies and passwords), so a one-time grant is
 * enough. Always Allow would only save a click across repeated imports, and
 * it writes a permanent ACL entry for a one-off onboarding step.
 */
export function MacKeychainNotice() {
  return (
    <div className="mb-4 rounded-xl border border-accent/25 bg-accent-tint p-4">
      <div className="flex items-start gap-3">
        <Lock className="mt-0.5 size-[18px] shrink-0 text-accent" />
        <div className="text-[12.5px] text-ink-2 leading-[1.5]">
          <span className="font-semibold text-ink">
            macOS will ask for your password
          </span>{' '}
          to read Chrome&rsquo;s saved data. That&rsquo;s expected. It&rsquo;s
          how your cookies and passwords get copied. Enter it and click{' '}
          <span className="font-semibold text-ink">Allow</span>.
        </div>
      </div>
      {/* Must stay a literal public path: importing the asset or inlining it
          breaks scripts/verify-chromium-build.ts (allowlist, no data: URLs).
          Intrinsic size is the native 832x334 at 1:2, so it renders crisp on
          retina; width/height keep the step column from shifting on load. */}
      <img
        alt="The macOS Keychain prompt, with the Allow button highlighted."
        className="mt-3 w-full max-w-[416px] rounded-lg border border-border-2"
        height={167}
        src="/icon/keychain-prompt.png"
        width={416}
      />
    </div>
  )
}
