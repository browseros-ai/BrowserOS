import type { CockpitStats } from '@browseros/claw-api'
import { createQuery } from 'react-query-kit'
import { apiClient } from './client'

// Final projection runs after live teardown; polling lets an already-mounted
// idle Cockpit observe the aggregate as soon as it lands.
export const useCockpitStats = createQuery<CockpitStats>({
  queryKey: ['api', 'cockpit', 'stats'],
  fetcher: async () => (await apiClient()).getCockpitStats(),
  refetchInterval: 3000,
})
