import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { type EvalConfig, EvalConfigSchema } from '../types'

// Re-export for backward compatibility
export { EvalConfigSchema }
export type ValidatedEvalConfig = EvalConfig

interface ValidationResult {
  valid: boolean
  config?: ValidatedEvalConfig
  errors: string[]
  warnings: string[]
}

export async function validateConfig(
  configPath: string,
): Promise<ValidationResult> {
  const errors: string[] = []
  const warnings: string[] = []

  let rawConfig: unknown
  try {
    const content = await Bun.file(configPath).text()
    rawConfig = JSON.parse(content)
  } catch (e) {
    return {
      valid: false,
      errors: [
        `Failed to read/parse config: ${e instanceof Error ? e.message : String(e)}`,
      ],
      warnings: [],
    }
  }

  const parseResult = EvalConfigSchema.safeParse(rawConfig)
  if (!parseResult.success) {
    const zodErrors = parseResult.error.errors.map(
      (e) => `${e.path.join('.')}: ${e.message}`,
    )
    return {
      valid: false,
      errors: ['Config schema validation failed:', ...zodErrors],
      warnings: [],
    }
  }

  const config = parseResult.data

  // Check if API key env vars are set (apiKey field contains env var name)
  const envVarsToCheck: string[] = []
  const isEnvVarName = (s: string) => /^[A-Z][A-Z0-9_]*$/.test(s)

  if (config.agent.type === 'single') {
    // Skip API key check for browseros provider (uses server's built-in auth)
    if (
      config.agent.provider !== LLM_PROVIDERS.BROWSEROS &&
      config.agent.apiKey &&
      isEnvVarName(config.agent.apiKey)
    ) {
      envVarsToCheck.push(config.agent.apiKey)
    }
  } else {
    const { orchestrator, executor } = config.agent
    if (orchestrator.apiKey && isEnvVarName(orchestrator.apiKey)) {
      envVarsToCheck.push(orchestrator.apiKey)
    }
    if (executor.apiKey && isEnvVarName(executor.apiKey)) {
      envVarsToCheck.push(executor.apiKey)
    }
  }

  for (const envVar of [...new Set(envVarsToCheck)]) {
    if (!process.env[envVar]) {
      errors.push(`Environment variable not set: ${envVar}`)
    }
  }

  // Server health check skipped — eval manages Chrome+Server lifecycle per worker

  if (config.num_workers > 5) {
    warnings.push(
      `num_workers=${config.num_workers} will create many browser windows`,
    )
  }

  return {
    valid: errors.length === 0,
    config: errors.length === 0 ? config : undefined,
    errors,
    warnings,
  }
}

export function printValidationResult(result: ValidationResult): void {
  if (result.valid) {
    console.log('Configuration is valid\n')
  } else {
    console.log('Configuration validation failed\n')
  }

  if (result.errors.length > 0) {
    console.log('Errors:')
    for (const e of result.errors) {
      console.log(`  - ${e}`)
    }
    console.log()
  }

  if (result.warnings.length > 0) {
    console.log('Warnings:')
    for (const w of result.warnings) {
      console.log(`  - ${w}`)
    }
    console.log()
  }
}
