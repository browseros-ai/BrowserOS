import { Cloud, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useState } from 'react'
import { useNavigate } from 'react-router'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { signInHintDismissedAtStorage } from '@/lib/onboarding/onboardingStorage'

const LONG_DISMISS_DURATION = 90 * 24 * 60 * 60 * 1000

export const SignInHint = () => {
  const { sessionInfo } = useSessionInfo()
  const isLoggedIn = !!sessionInfo?.user
  const navigate = useNavigate()
  const [dismissed, setDismissed] = useState(false)
  const [dontAskAgain, setDontAskAgain] = useState(false)

  const handleDismiss = async () => {
    setDismissed(true)
    const dismissUntil = dontAskAgain
      ? Date.now() + LONG_DISMISS_DURATION
      : Date.now()
    await signInHintDismissedAtStorage.setValue(dismissUntil)
  }

  const show = !dismissed && !isLoggedIn

  return (
    <AnimatePresence>
      {show && (
        <motion.div
          className="fixed right-4 bottom-4 z-50"
          initial={{ opacity: 0, x: 100 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: 100 }}
          transition={{ type: 'spring', damping: 25, stiffness: 300 }}
        >
          <Card className="w-80 gap-0 py-4">
            <CardHeader className="gap-3">
              <div className="flex items-start justify-between">
                <div className="flex items-center gap-2">
                  <Cloud className="size-5 text-muted-foreground" />
                  <CardTitle className="text-base">Sync your data</CardTitle>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7 shrink-0"
                  onClick={handleDismiss}
                >
                  <X className="size-4" />
                </Button>
              </div>
              <CardDescription>
                Sign in to sync conversation history to the cloud.
              </CardDescription>
              <label
                htmlFor="sync-dont-ask-again"
                className="flex items-center gap-2 text-muted-foreground text-sm"
              >
                <Checkbox
                  id="sync-dont-ask-again"
                  checked={dontAskAgain}
                  onCheckedChange={(checked) =>
                    setDontAskAgain(checked === true)
                  }
                />
                Don't ask again
              </label>
              <Button className="w-full" onClick={() => navigate('/login')}>
                Sign in
              </Button>
            </CardHeader>
          </Card>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
