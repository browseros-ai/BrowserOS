import { useQueryClient } from '@tanstack/react-query'
import { clear } from 'idb-keyval'
import { Loader2 } from 'lucide-react'
import type { FC } from 'react'
import { useEffect } from 'react'
import { useNavigate } from 'react-router'
import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { resetIdentity } from '@/lib/analytics/identify'
import { signOut } from '@/lib/auth/auth-client'
import { providersStorage } from '@/lib/llm-providers/storage'
import { scheduledJobStorage } from '@/lib/schedules/scheduleStorage'

export const LogoutPage: FC = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  // biome-ignore lint/correctness/useExhaustiveDependencies: must run only once to ensure the logout process happens successfully
  useEffect(() => {
    const performLogout = async () => {
      await providersStorage.removeValue()
      await scheduledJobStorage.removeValue()
      queryClient.clear()
      await clear()

      resetIdentity()
      await signOut()
      navigate('/home', { replace: true })
    }

    performLogout()
  }, [])

  return (
    <Card className="w-full max-w-md">
      <CardHeader className="text-center">
        <div className="mx-auto mb-4">
          <Loader2 className="size-8 animate-spin text-muted-foreground" />
        </div>
        <CardTitle className="text-2xl">Logging out</CardTitle>
        <CardDescription>
          Clearing your session and synced data...
        </CardDescription>
      </CardHeader>
    </Card>
  )
}
