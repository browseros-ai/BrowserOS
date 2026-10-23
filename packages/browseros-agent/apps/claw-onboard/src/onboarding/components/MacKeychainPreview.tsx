import { cn } from '@/lib/utils'
import { NeoMark } from './NeoMark'

const BUTTON =
  'flex h-6 shrink-0 items-center rounded-[7px] border border-[#CBD5E1] bg-white px-3 font-medium text-[12px] leading-[14px]'

/**
 * An inert, flat drawing of the OS-owned Keychain prompt with Allow ringed
 * (Paper 2b); credentials only ever go to macOS. It is drawn at twice its
 * 228×104 size, so the caller halves it with `zoom`.
 */
export function MacKeychainPreview({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        'flex h-[208px] w-[456px] flex-col gap-3 rounded-2xl border-2 border-[#CBD5E1] bg-[#F4F6FA] px-5 pt-[18px] pb-4 font-sans text-[#1D1F24]',
        className,
      )}
    >
      <div className="flex gap-3.5">
        <div className="relative h-[54px] w-[50px] shrink-0">
          <svg aria-hidden width="50" height="54" viewBox="0 0 52 56">
            <path
              d="M15 24v-7a11 11 0 0 1 22 0v7"
              fill="none"
              stroke="#9AA1AE"
              strokeWidth="5"
            />
            <rect
              x="6"
              y="22"
              width="40"
              height="30"
              rx="6"
              fill="#CBD5E1"
              stroke="#9AA1AE"
              strokeWidth="1.5"
            />
            <circle cx="26" cy="35" r="3.5" fill="#717783" />
            <path
              d="M26 37v6"
              fill="none"
              stroke="#717783"
              strokeWidth="3"
              strokeLinecap="round"
            />
          </svg>
          <NeoMark className="absolute top-[34px] left-[30px] size-[22px] rounded-[5px]" />
        </div>
        <div className="flex w-[352px] shrink-0 flex-col gap-1.5">
          <p className="m-0 font-bold text-[13.2px] leading-[17.6px]">
            BrowserOS neo Helper wants to use your confidential information
            stored in &ldquo;Chrome Safe Storage&rdquo; in your keychain.
          </p>
          <p className="m-0 text-[#4A4F58] text-[11.6px] leading-4">
            To allow this, enter the &ldquo;login&rdquo; keychain password.
          </p>
        </div>
      </div>
      <div className="flex items-center gap-2 pl-16 text-[12px] leading-[14px]">
        Password:
        <span className="h-[22px] flex-1 rounded-md border-2 border-[#CBD5E1] bg-white" />
      </div>
      <div className="mt-auto flex items-center gap-2">
        <span className="grid size-6 shrink-0 place-items-center rounded-full border border-[#CBD5E1] bg-white font-medium text-[12px] leading-[14px]">
          ?
        </span>
        <span className={BUTTON}>Always Allow</span>
        <span className="flex-1" />
        <span className={BUTTON}>Deny</span>
        <span className="flex h-6 shrink-0 items-center rounded-[7px] bg-[#0254EC] px-4 font-semibold text-[12px] text-white leading-[14px] shadow-[0_0_0_3px_#F4F6FA,0_0_0_6px_#0254EC,0_0_0_12px_#0254EC29]">
          Allow
        </span>
      </div>
    </div>
  )
}
