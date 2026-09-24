import { storage } from '@wxt-dev/storage'
import { createQuery } from 'react-query-kit'

export const hostedModelRetiredStorage = storage.defineItem<boolean>(
  'local:hosted-model-retired',
  { fallback: false },
)

/**
 * Whether to explain the retirement rather than invite a first setup.
 *
 * Absent means "not known to be a returning user", which is the safe direction
 * to be wrong in: a new user told to get started reads correctly either way.
 */
export const useHostedModelRetired = createQuery<boolean>({
  queryKey: ['hosted-model-retired'],
  fetcher: () => hostedModelRetiredStorage.getValue(),
})
