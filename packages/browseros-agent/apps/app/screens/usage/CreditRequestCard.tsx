import { Check, Copy, Gift } from 'lucide-react'
import {
  type ChangeEvent,
  type FC,
  type FormEvent,
  useEffect,
  useRef,
  useState,
} from 'react'
import { toast } from 'sonner'
import ProductHuntLogo from '@/assets/producthunt.svg'
import { PRODUCT_HUNT_URL } from '@/components/promo/ProductHuntBanner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { discordUrl } from '@/lib/constants/productUrls'
import { sentry } from '@/lib/sentry/sentry'
import {
  DISCORD_HANDLE_MAX_LENGTH,
  useSubmitCreditRequest,
} from './credit-request.hooks'

const COPIED_RESET_MS = 2000

export interface CreditRequestCardProps {
  browserosId?: string
}

export const CreditRequestCard: FC<CreditRequestCardProps> = ({
  browserosId,
}) => {
  const [discordHandle, setDiscordHandle] = useState('')
  const [isCopied, setIsCopied] = useState(false)
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout>>(undefined)
  const submission = useSubmitCreditRequest()

  useEffect(() => () => clearTimeout(copiedTimerRef.current), [])

  // An empty id is as bad as a missing one: a user who copies it and pastes it
  // into Discord gets nothing back, so both collapse into the fallback below.
  const id = browserosId?.trim()
  const handle = discordHandle.trim()

  const handleOpenProductHunt = () => {
    chrome.tabs.create({ url: PRODUCT_HUNT_URL })
  }

  const handleCopy = async () => {
    if (!id) return
    try {
      await navigator.clipboard.writeText(id)
      setIsCopied(true)
      clearTimeout(copiedTimerRef.current)
      copiedTimerRef.current = setTimeout(
        () => setIsCopied(false),
        COPIED_RESET_MS,
      )
    } catch (error) {
      sentry.captureException(error, {
        extra: { message: 'Failed to copy the BrowserOS ID to the clipboard' },
      })
      toast.error('Could not copy — select the ID and copy it manually.')
    }
  }

  const handleChange = (event: ChangeEvent<HTMLInputElement>) => {
    setDiscordHandle(event.target.value)
    if (submission.isSuccess || submission.isError) submission.reset()
  }

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!id || !handle || submission.isPending) return
    submission.mutate(
      { browserosId: id, discordHandle: handle },
      { onSuccess: () => setDiscordHandle('') },
    )
  }

  return (
    <div className="space-y-4 rounded-xl border p-5">
      <div className="flex items-center gap-3">
        <Gift className="h-5 w-5 text-muted-foreground" />
        <div>
          <p className="font-semibold text-sm">
            Upvote us and get free credits
          </p>
          <p className="text-muted-foreground text-xs">
            Upvote BrowserOS on Product Hunt and we top up your credits by hand.
          </p>
        </div>
      </div>

      <Button
        size="sm"
        onClick={handleOpenProductHunt}
        className="gap-2 bg-[#ff6154] text-white hover:bg-[#e5563f]"
      >
        <img src={ProductHuntLogo} alt="" className="h-4 w-4" />
        Upvote on Product Hunt
      </Button>

      {id ? (
        <>
          <div className="space-y-1.5">
            <p className="font-medium text-xs">Your BrowserOS ID</p>
            <div className="flex items-center gap-2">
              <div className="min-w-0 flex-1 select-all break-all rounded-lg border bg-background px-3 py-2 font-mono text-xs">
                {id}
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={handleCopy}
                className="shrink-0 gap-1.5"
              >
                {isCopied ? (
                  <>
                    <Check className="h-3.5 w-3.5 text-green-600" />
                    Copied
                  </>
                ) : (
                  <>
                    <Copy className="h-3.5 w-3.5" />
                    Copy
                  </>
                )}
              </Button>
            </div>
          </div>

          <form onSubmit={handleSubmit} className="space-y-1.5">
            <Label htmlFor="discord-handle" className="font-medium text-xs">
              Your Discord handle
            </Label>
            <div className="flex items-center gap-2">
              <Input
                id="discord-handle"
                value={discordHandle}
                onChange={handleChange}
                maxLength={DISCORD_HANDLE_MAX_LENGTH}
                placeholder="yourhandle"
                disabled={submission.isPending}
                className="flex-1"
              />
              <Button
                type="submit"
                size="sm"
                disabled={!handle || submission.isPending}
                className="shrink-0"
              >
                {submission.isPending ? 'Sending...' : 'Submit'}
              </Button>
            </div>
            <div aria-live="polite">
              {submission.isSuccess && (
                <p className="text-green-600 text-xs dark:text-green-500">
                  Sent — DM us on Discord with your screenshot.
                </p>
              )}
              {submission.isError && (
                <p className="text-destructive text-xs">
                  {submission.error.message}
                </p>
              )}
            </div>
          </form>
        </>
      ) : (
        <p className="rounded-lg bg-muted/50 px-3 py-2.5 text-muted-foreground text-xs">
          Your BrowserOS ID isn&apos;t available right now — reopen this page or
          restart BrowserOS, then try again.
        </p>
      )}

      <p className="text-muted-foreground text-xs">
        Upvote on Product Hunt, then{' '}
        <a
          href={discordUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="font-medium text-foreground underline underline-offset-2"
        >
          DM us on Discord
        </a>{' '}
        with a screenshot and your BrowserOS ID.
      </p>
    </div>
  )
}
