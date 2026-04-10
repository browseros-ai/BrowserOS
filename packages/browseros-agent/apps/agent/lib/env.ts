import { ZodError, z } from 'zod'

const EnvSchema = z.object({
  VITE_BROWSEROS_SERVER_PORT: z.coerce.number().optional(),
  VITE_PUBLIC_POSTHOG_KEY: z.string().optional(),
  VITE_PUBLIC_POSTHOG_HOST: z.string().optional(),
  VITE_PUBLIC_SENTRY_DSN: z.string().optional(),
  VITE_PUBLIC_BROWSEROS_API: z.string().optional(),
  PROD: z.boolean(),
})

try {
  EnvSchema.parse(import.meta.env)
} catch (error) {
  if (error instanceof ZodError) {
    let message = 'Missing required values in .env:\n'
    for (const issue of error.issues) {
      message += `${issue.path.join('.')}\n`
    }
    const e = new Error(message)
    e.stack = ''
    throw e
  }
  // biome-ignore lint/suspicious/noConsole: allowed to display error information
  console.error(error)
  throw error
}

/**
 * @public
 */
export const env = EnvSchema.parse(import.meta.env)
