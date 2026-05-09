import {
  AlertCircle,
  ArrowLeft,
  CheckCircle2,
  Loader2,
  Mail,
} from 'lucide-react'
import type { FC } from 'react'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router'
import { CloudSyncDisclosure } from '@/components/auth/CloudSyncDisclosure'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Separator } from '@/components/ui/separator'
import { signIn, useSession } from '@/lib/auth/auth-client'

type LoginState = 'idle' | 'loading' | 'magic-link-sent' | 'error'

export const LoginPage: FC = () => {
  const navigate = useNavigate()
  const { data: session, isPending } = useSession()
  const [email, setEmail] = useState('')
  const [state, setState] = useState<LoginState>('idle')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (session && !isPending) {
      navigate('/home', { replace: true })
    }
  }, [session, isPending, navigate])

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
    } catch (err) {
      setState('error')
      setError(err instanceof Error ? err.message : 'Failed to send magic link')
    }
  }

  const handleGoogleSignIn = async () => {
    setState('loading')
    setError(null)

    try {
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

  if (isPending) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <Loader2 className="size-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (state === 'magic-link-sent') {
    return (
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="mx-auto mb-4 flex size-12 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/30">
            <CheckCircle2 className="size-6 text-green-600 dark:text-green-400" />
          </div>
          <CardTitle>Check your email</CardTitle>
          <CardDescription>
            We sent a magic link to <strong>{email}</strong>
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <p className="text-center text-muted-foreground text-sm">
            Click the link in the email to sign in. If you don't see it, check
            your spam folder.
          </p>
          <Button
            variant="outline"
            className="w-full"
            onClick={() => {
              setState('idle')
              setEmail('')
            }}
          >
            Use a different email
          </Button>
        </CardContent>
      </Card>
    )
  }

  return (
    <Card className="w-full max-w-md">
      <CardHeader>
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => navigate(-1)}
            className="shrink-0"
          >
            <ArrowLeft className="size-4" />
          </Button>
          <div className="flex-1 pr-9 text-center">
            <CardTitle className="text-2xl">Welcome to BrowserOS</CardTitle>
            <CardDescription>
              Sign in to your account to continue
            </CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-6">
        {error && (
          <Alert variant="destructive">
            <AlertCircle className="size-4" />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <form onSubmit={handleMagicLink} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="email">Email</Label>
            <Input
              id="email"
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
            className="w-full"
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

        <div className="relative">
          <div className="absolute inset-0 flex items-center">
            <Separator className="w-full" />
          </div>
          <div className="relative flex justify-center text-xs uppercase">
            <span className="bg-card px-2 text-muted-foreground">
              Or continue with
            </span>
          </div>
        </div>

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

        <CloudSyncDisclosure />
      </CardContent>
    </Card>
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
