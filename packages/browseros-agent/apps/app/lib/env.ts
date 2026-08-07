import { ZodError, z } from 'zod'
import { parseBrowserOSApiUrl } from './browseros-api-url'

export function parseAlphaFeaturesFlag(value: string | undefined): boolean {
  return value === 'true'
}

const EnvSchema = z.object({
  VITE_ALPHA_FEATURES: z.string().optional().transform(parseAlphaFeaturesFlag),
  VITE_PUBLIC_POSTHOG_KEY: z.string().optional(),
  VITE_PUBLIC_POSTHOG_HOST: z.string().optional(),
  VITE_PUBLIC_SENTRY_DSN: z.string().optional(),
  VITE_PUBLIC_BROWSEROS_API: z
    .string()
    .optional()
    .transform(parseBrowserOSApiUrl),
  // Overrides the LLM gateway origin. Exists so a dev build can be pointed at a
  // locally-run gateway (`wrangler dev` on 8787); unset everywhere else, so
  // production keeps using the hardcoded default.
  VITE_PUBLIC_BROWSEROS_GATEWAY: z.string().optional(),
  PROD: z.boolean().optional().default(false),
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
