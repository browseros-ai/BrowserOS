import { useEffect, useRef } from 'react'
import useDeepCompareEffect from 'use-deep-compare-effect'
import { type McpServer, useMcpServers } from '@/lib/mcp/mcpServerStorage'
import { usePersonalization } from '@/lib/personalization/personalizationStorage'
import { useChatTargetSelection } from './use-chat-target-selection'

const constructMcpServers = (servers: McpServer[]) => {
  return servers
    .filter((eachServer) => eachServer.type === 'managed')
    .map((each) => each.managedServerName)
}

const constructCustomServers = (servers: McpServer[]) => {
  return servers
    .filter((eachServer) => eachServer.type === 'custom')
    .map((each) => ({
      name: each.displayName,
      url: each.config?.url,
    }))
}

export const useChatRefs = () => {
  const selection = useChatTargetSelection()
  const { servers: mcpServers } = useMcpServers()
  const { personalization } = usePersonalization()

  const enabledMcpServersRef = useRef(constructMcpServers(mcpServers))
  const enabledCustomServersRef = useRef(constructCustomServers(mcpServers))
  const personalizationRef = useRef(personalization)

  useDeepCompareEffect(() => {
    enabledMcpServersRef.current = constructMcpServers(mcpServers)
    enabledCustomServersRef.current = constructCustomServers(mcpServers)
  }, [mcpServers])

  useEffect(() => {
    personalizationRef.current = personalization
  }, [personalization])

  return {
    ...selection,
    enabledMcpServersRef,
    enabledCustomServersRef,
    personalizationRef,
  }
}
