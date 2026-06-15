import { useNavigate, useParams } from 'react-router'
import { useReplay } from '@/modules/api/replay.hooks'

export function useReplayData() {
  const { runId = '' } = useParams<{ runId: string }>()
  const { data: replay, isLoading } = useReplay({ variables: { runId } })
  const navigate = useNavigate()
  return { replay, runId, isLoading, navigate }
}
