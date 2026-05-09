import { AlertCircle, CheckCircle2, Loader2, Mail } from 'lucide-react'
import { useState } from 'react'
import { CloudSyncDisclosure } from '@/components/auth/CloudSyncDisclosure'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Separator } from '@/components/ui/separator'
import { signIn } from '@/lib/auth/auth-client'
import {
  ONBOARDING_SIGNIN_COMPLETED_EVENT,
  ONBOARDING_SIGNIN_SKIPPED_EVENT,
  ONBOARDING_STEP_COMPLETED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { authRedirectPathStorage } from '@/lib/onboarding/onboardingStorage'
import { type StepDirection, StepTransition } from './StepTransition'

interface StepTwoProps {
  direction: StepDirection
  onContinue: () => void
}

type SignInState = 'idle' | 'loading' | 'magic-link-sent' | 'error'

export const StepTwo = ({ direction, onContinue }: StepTwoProps) => {
  const [email, setEmail] = useState('')
  const [state, setState] = useState<SignInState>('idle')
  const [error, setError] = useState<string | null>(null)

  const handleSkip = () => {
    track(ONBOARDING_SIGNIN_SKIPPED_EVENT)
    track(ONBOARDING_STEP_COMPLETED_EVENT, {
      step: 4,
      step_name: 'signin',
      skipped: true,
    })
    onContinue()
  }

  const handleMagicLink = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!email.trim()) return

    setState('loading')
    setError(null)

    try {
      const result = await signIn.magicLink({
        email: email.trim(),
        callbackURL: '/home',
      })

      if (result.error) {
        setState('error')
        setError(result.error.message || 'Failed to send magic link')
        return
      }

      setState('magic-link-sent')
      track(ONBOARDING_SIGNIN_COMPLETED_EVENT, { method: 'magic_link' })
      track(ONBOARDING_STEP_COMPLETED_EVENT, { step: 4, step_name: 'signin' })
    } catch (err) {
      setState('error')
      setError(err instanceof Error ? err.message : 'Failed to send magic link')
    }
  }

  const handleGoogleSignIn = async () => {
    setState('loading')
    setError(null)

    try {
      track(ONBOARDING_SIGNIN_COMPLETED_EVENT, { method: 'google' })
      track(ONBOARDING_STEP_COMPLETED_EVENT, { step: 4, step_name: 'signin' })

      await authRedirectPathStorage.setValue('/onboarding/demo')
      await signIn.social({
        provider: 'google',
        callbackURL: '/home',
      })
    } catch (err) {
      setState('error')
      setError(
        err instanceof Error ? err.message : 'Failed to sign in with Google',
      )
    }
  }

  if (state === 'magic-link-sent') {
    return (
      <StepTransition direction={direction}>
        <div className="flex h-full flex-col items-center justify-center">
          <div className="w-full max-w-md space-y-6 text-center">
            <div className="mx-auto flex size-12 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/30">
              <CheckCircle2 className="size-6 text-green-600 dark:text-green-400" />
            </div>
            <div className="space-y-2">
              <h2 className="font-bold text-2xl tracking-tight">
                Check your email
              </h2>
              <p className="text-muted-foreground text-sm">
                We sent a magic link to <strong>{email}</strong>. Click the link
                to sign in.
              </p>
            </div>
            <div className="flex flex-col gap-3">
              <Button
                variant="outline"
                onClick={() => {
                  setState('idle')
                  setEmail('')
                }}
              >
                Use a different email
              </Button>
              <Button
                variant="ghost"
                onClick={onContinue}
                className="text-muted-foreground"
              >
                Continue without signing in
              </Button>
            </div>
          </div>
        </div>
      </StepTransition>
    )
  }

  return (
    <StepTransition direction={direction}>
      <div className="flex h-full flex-col items-center justify-center">
        <div className="w-full max-w-md space-y-6">
          <div className="space-y-2 text-center">
            <h2 className="font-bold text-3xl tracking-tight">
              Sign in to BrowserOS
            </h2>
            <p className="text-base text-muted-foreground">
              Sync your settings and unlock cloud features
            </p>
          </div>

          {error && (
            <Alert variant="destructive">
              <AlertCircle className="size-4" />
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}

          <Button
            variant="outline"
            className="w-full"
            onClick={handleGoogleSignIn}
            disabled={state === 'loading'}
          >
            {state === 'loading' ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <GoogleIcon />
            )}
            Continue with Google
          </Button>

          <div className="relative">
            <div className="absolute inset-0 flex items-center">
              <Separator className="w-full" />
            </div>
            <div className="relative flex justify-center text-xs uppercase">
              <span className="bg-background px-2 text-muted-foreground">
                Or continue with email
              </span>
            </div>
          </div>

          <form onSubmit={handleMagicLink} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="signin-email">Email</Label>
              <Input
                id="signin-email"
                type="email"
                placeholder="you@example.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                disabled={state === 'loading'}
                required
              />
            </div>
            <Button
              type="submit"
              className="w-full bg-[var(--accent-orange)] text-white hover:bg-[var(--accent-orange)]/90"
              disabled={state === 'loading' || !email.trim()}
            >
              {state === 'loading' ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Mail className="size-4" />
              )}
              Send Magic Link
            </Button>
          </form>

          <CloudSyncDisclosure />

          <div className="text-center">
            <Button
              variant="ghost"
              onClick={handleSkip}
              className="text-muted-foreground"
            >
              Skip for now
            </Button>
          </div>
        </div>
      </div>
    </StepTransition>
  )
}

function GoogleIcon() {
  return (
    <svg viewBox="0 0 24 24" className="size-4" role="img" aria-label="Google">
      <path
        d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"
        fill="#4285F4"
      />
      <path
        d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
        fill="#34A853"
      />
      <path
        d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
        fill="#FBBC05"
      />
      <path
        d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
        fill="#EA4335"
      />
    </svg>
  )
}
