import { Gift } from 'lucide-react'
import { type ChangeEvent, type FC, type FormEvent, useState } from 'react'
import ProductHuntLogo from '@/assets/producthunt.svg'
import { openProductHuntFocused } from '@/components/promo/ProductHuntBanner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { discordUrl } from '@/lib/constants/productUrls'
import {
  DISCORD_HANDLE_MAX_LENGTH,
  useSubmitCreditRequest,
} from './credit-request.hooks'

export interface CreditRequestCardProps {
  browserosId?: string
}

export const CreditRequestCard: FC<CreditRequestCardProps> = ({
  browserosId,
}) => {
  const [discordHandle, setDiscordHandle] = useState('')
  const submission = useSubmitCreditRequest()

  // The id is never shown; it only identifies this browser to the gateway. An
  // empty one is as useless as a missing one, so both collapse into the
  // fallback below rather than submitting an unmatchable request.
  const id = browserosId?.trim()
  const handle = discordHandle.trim()

  const handleOpenProductHunt = () => {
    openProductHuntFocused()
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
            Upvote us and get 100 free credits
          </p>
          <p className="text-muted-foreground text-xs">
            Upvote and comment on Product Hunt, and we add 100 credits to your
            account.
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
      ) : (
        <p className="rounded-lg bg-muted/50 px-3 py-2.5 text-muted-foreground text-xs">
          We could not identify your browser right now — reopen this page or
          restart BrowserOS, then try again.
        </p>
      )}

      <p className="text-muted-foreground text-xs">
        Upvote and comment on Product Hunt, then{' '}
        <a
          href={discordUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="font-medium text-foreground underline underline-offset-2"
        >
          DM us on Discord
        </a>{' '}
        with a screenshot.
      </p>
    </div>
  )
}
