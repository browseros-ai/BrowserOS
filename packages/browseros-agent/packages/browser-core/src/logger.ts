import type { LoggerInterface } from '@browseros/shared/types/logger'

export const logger: LoggerInterface = {
  debug(message, meta) {
    console.debug(message, meta ?? '')
  },
  info(message, meta) {
    console.info(message, meta ?? '')
  },
  warn(message, meta) {
    console.warn(message, meta ?? '')
  },
  error(message, meta) {
    console.error(message, meta ?? '')
  },
}
