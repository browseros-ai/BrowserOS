import { type ClassValue, clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'

/**
 * @public
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
