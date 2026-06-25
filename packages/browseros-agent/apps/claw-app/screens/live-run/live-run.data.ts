import { useNavigate, useParams } from 'react-router'
import { useRun } from '@/modules/api/run.hooks'

export function useLiveRunData() {
  const { runId = '' } = useParams<{ runId: string }>()
  const { data: run, isLoading } = useRun({ variables: { runId } })
  const navigate = useNavigate()
  return { run, runId, isLoading, navigate }
}
