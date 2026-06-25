import { createAuthClient } from 'better-auth/react'
import { env } from '../env'

export const { signIn, signOut, useSession } = createAuthClient({
  baseURL: env.VITE_PUBLIC_BROWSEROS_API,
})
