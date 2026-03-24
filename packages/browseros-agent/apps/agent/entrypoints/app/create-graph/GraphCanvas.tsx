import cytoscape from 'cytoscape'
import dagre from 'cytoscape-dagre'
// @ts-expect-error no types available
import nodeHtmlLabel from 'cytoscape-node-html-label'
import DOMPurify from 'dompurify'
import {
  ArrowLeft,
  Maximize,
  Minus,
  Pencil,
  Play,
  Plus,
  Save,
} from 'lucide-react'
import type { FC } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate } from 'react-router'
import useDeepCompareEffect from 'use-deep-compare-effect'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { getProductLogoUrl } from '@/lib/branding/logo'
import type { GraphData } from './CreateGraph'
import type { NodeType } from './CustomNode'

const PRODUCT_LOGO_URL = getProductLogoUrl()

cytoscape.use(dagre)
nodeHtmlLabel(cytoscape)

const NODE_CONFIG: Record<
  NodeType,
  { color: string; bgColor: string; icon: string; label: string }
> = {
  start: {
    color: '#22c55e',
    bgColor: 'rgba(34, 197, 94, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="6 3 20 12 6 21 6 3"></polygon></svg>`,
    label: 'START',
  },
  end: {
    color: '#ef4444',
    bgColor: 'rgba(239, 68, 68, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"></rect></svg>`,
    label: 'END',
  },
  nav: {
    color: '#3b82f6',
    bgColor: 'rgba(59, 130, 246, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="3 11 22 2 13 21 11 13 3 11"></polygon></svg>`,
    label: 'NAVIGATE',
  },
  act: {
    color: '#8b5cf6',
    bgColor: 'rgba(139, 92, 246, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m4 4 7.07 17 2.51-7.39L21 11.07z"></path></svg>`,
    label: 'ACTION',
  },
  extract: {
    color: '#f59e0b',
    bgColor: 'rgba(245, 158, 11, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path><polyline points="7 10 12 15 17 10"></polyline><line x1="12" x2="12" y1="15" y2="3"></line></svg>`,
    label: 'EXTRACT',
  },
  verify: {
    color: '#10b981',
    bgColor: 'rgba(16, 185, 129, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"></path><polyline points="22 4 12 14.01 9 11.01"></polyline></svg>`,
    label: 'VERIFY',
  },
  decision: {
    color: '#ec4899',
    bgColor: 'rgba(236, 72, 153, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="6" x2="6" y1="3" y2="15"></line><circle cx="18" cy="6" r="3"></circle><circle cx="6" cy="18" r="3"></circle><path d="M18 9a9 9 0 0 1-9 9"></path></svg>`,
    label: 'DECISION',
  },
  loop: {
    color: '#06b6d4',
    bgColor: 'rgba(6, 182, 212, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"></path><path d="M21 3v5h-5"></path></svg>`,
    label: 'LOOP',
  },
  fork: {
    color: '#6366f1',
    bgColor: 'rgba(99, 102, 241, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5"></path><path d="M8 3H3v5"></path><path d="M12 22v-8.3a4 4 0 0 0-1.172-2.872L3 3"></path><path d="m15 9 6-6"></path></svg>`,
    label: 'FORK',
  },
  join: {
    color: '#84cc16',
    bgColor: 'rgba(132, 204, 22, 0.1)',
    icon: `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="18" cy="18" r="3"></circle><circle cx="6" cy="6" r="3"></circle><path d="M6 21V9a9 9 0 0 0 9 9"></path></svg>`,
    label: 'JOIN',
  },
}

const initialData: GraphData = {
  nodes: [
    {
      id: 'start',
      type: 'start',
      data: { label: 'Use the Chat to build your workflow!' },
    },
  ],
  edges: [],
}

const MIN_NODE_WIDTH = 180
const MAX_NODE_WIDTH = 240
const BASE_NODE_HEIGHT = 70
const CHAR_WIDTH = 7
const ICON_AND_PADDING = 62
const MAX_ZOOM = 1.2

const calculateNodeDimensions = (
  label: string,
): { width: number; height: number } => {
  const textWidth = label.length * CHAR_WIDTH + ICON_AND_PADDING
  const width = Math.max(MIN_NODE_WIDTH, Math.min(MAX_NODE_WIDTH, textWidth))

  const maxCharsPerLine = Math.floor((width - ICON_AND_PADDING) / CHAR_WIDTH)
  const lines = Math.ceil(label.length / maxCharsPerLine)
  const extraHeight = Math.max(0, lines - 1) * 18
  const height = BASE_NODE_HEIGHT + extraHeight

  return { width, height }
}

const createNodeHtml = (type: NodeType, label: string): string => {
  const config = NODE_CONFIG[type] || NODE_CONFIG.start
  const sanitizedLabel = DOMPurify.sanitize(label, { ALLOWED_TAGS: [] })
  return `
    <div class="graph-node" style="
      display: flex;
      align-items: flex-start;
      gap: 10px;
      min-width: 160px;
      max-width: 220px;
      padding: 12px 16px;
      background-color: var(--graph-node-bg);
      border: 1px solid var(--graph-node-border);
      border-radius: 10px;
      box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
      font-family: system-ui, -apple-system, sans-serif;
    ">
      <div style="
        flex-shrink: 0;
        color: ${config.color};
        margin-top: 2px;
      ">
        ${config.icon}
      </div>
      <div style="flex: 1; min-width: 0;">
        <div style="
          font-size: 10px;
          font-weight: 600;
          letter-spacing: 0.05em;
          color: ${config.color};
          margin-bottom: 4px;
        ">${config.label}</div>
        <div style="
          font-size: 13px;
          font-weight: 500;
          color: var(--graph-node-text);
          line-height: 1.4;
          word-wrap: break-word;
        ">${sanitizedLabel}</div>
      </div>
    </div>
  `
}

type GraphCanvasProps = {
  graphName: string
  onGraphNameChange: (name: string) => void
  graphData?: GraphData
  codeId?: string
  onClickTest: () => unknown
  onClickSave: () => unknown
  isSaved: boolean
  hasUnsavedChanges: boolean
  shouldBlockNavigation: boolean
  panelSize?: { asPercentage: number; inPixels: number }
}

export const GraphCanvas: FC<GraphCanvasProps> = ({
  graphName,
  onGraphNameChange,
  graphData = initialData,
  codeId,
  onClickTest,
  onClickSave,
  isSaved,
  hasUnsavedChanges,
  shouldBlockNavigation,
  panelSize,
}) => {
  const [isEditingName, setIsEditingName] = useState(false)
  const navigate = useNavigate()
  const containerRef = useRef<HTMLDivElement>(null)
  const cyRef = useRef<cytoscape.Core | null>(null)

  const handleBack = () => {
    if (shouldBlockNavigation) {
      const confirmed = window.confirm(
        'You have unsaved changes. Are you sure you want to leave?',
      )
      if (!confirmed) return
    }
    navigate(-1)
  }

  const canTest = !!codeId
  const canSave = !!graphName && !!codeId && hasUnsavedChanges

  const getTestTooltip = () => {
    if (!codeId) return 'Create a workflow using the chat first'
    return 'Run a test of this workflow'
  }

  const getSaveTooltip = () => {
    if (!codeId) return 'Create a workflow using the chat first'
    if (!graphName) return 'Provide a name for the workflow'
    if (isSaved && !hasUnsavedChanges) return 'Workflow already saved'
    return isSaved ? 'Save changes to this workflow' : 'Save this workflow'
  }

  const getSaveButtonLabel = () => {
    return isSaved ? 'Save Changes' : 'Save Workflow'
  }

  const zoomIn = useCallback(() => {
    cyRef.current?.zoom(cyRef.current.zoom() * 1.2)
    cyRef.current?.center()
  }, [])

  const zoomOut = useCallback(() => {
    cyRef.current?.zoom(cyRef.current.zoom() / 1.2)
    cyRef.current?.center()
  }, [])

  const fitView = useCallback(() => {
    cyRef.current?.fit(undefined, 50)
    cyRef.current?.center()
  }, [])

  useEffect(() => {
    if (!containerRef.current) return

    const cy = cytoscape({
      container: containerRef.current,
      elements: [],
      style: [
        {
          selector: 'node',
          style: {
            width: 'data(nodeWidth)',
            height: 'data(nodeHeight)',
            'background-opacity': 0,
            'border-width': 0,
          },
        },
        {
          selector: 'edge',
          style: {
            width: 2,
            'line-color': '#f97316',
            'target-arrow-color': '#f97316',
            'target-arrow-shape': 'triangle',
            'curve-style': 'bezier',
            'arrow-scale': 1.2,
          },
        },
        {
          selector: 'edge.back-edge',
          style: {
            'line-style': 'dashed',
            'line-dash-pattern': [6, 3],
            'curve-style': 'unbundled-bezier',
            'control-point-distances': [100],
            'control-point-weights': [0.5],
          },
        },
      ],
      layout: { name: 'preset' },
      userZoomingEnabled: true,
      userPanningEnabled: true,
      boxSelectionEnabled: false,
      selectionType: 'single',
      autoungrabify: true,
      autounselectify: true,
      maxZoom: MAX_ZOOM,
      minZoom: 0.2,
    })

    // @ts-expect-error nodeHtmlLabel extension
    cy.nodeHtmlLabel([
      {
        query: 'node',
        halign: 'center',
        valign: 'center',
        halignBox: 'center',
        valignBox: 'center',
        tpl: (data: { type: NodeType; label: string }) => {
          return createNodeHtml(data.type, data.label)
        },
      },
    ])

    cyRef.current = cy

    return () => {
      cy.destroy()
    }
  }, [])

  const updateGraph = useCallback((data: GraphData) => {
    const cy = cyRef.current
    if (!cy) return

    cy.elements().remove()

    const nodes = data.nodes.map((node) => {
      const dimensions = calculateNodeDimensions(node.data.label)
      return {
        data: {
          id: node.id,
          label: node.data.label,
          type: node.type as NodeType,
          nodeWidth: dimensions.width,
          nodeHeight: dimensions.height,
        },
      }
    })

    const edges = data.edges.map((edge) => ({
      data: {
        id: edge.id,
        source: edge.source,
        target: edge.target,
      },
    }))

    cy.add([...nodes, ...edges])

    cy.layout({
      name: 'dagre',
      rankDir: 'TB',
      nodeSep: 80,
      rankSep: 100,
      padding: 50,
      animate: true,
      animationDuration: 300,
      fit: true,
    } as cytoscape.LayoutOptions).run()

    setTimeout(() => {
      cy.edges().forEach((edge) => {
        const sourceNode = edge.source()
        const targetNode = edge.target()
        const sourceY = sourceNode.position('y')
        const targetY = targetNode.position('y')

        if (sourceY > targetY) {
          edge.addClass('back-edge')
        }
      })
    }, 350)
  }, [])

  useDeepCompareEffect(() => {
    updateGraph(graphData)
  }, [graphData])

  useEffect(() => {
    if (panelSize?.inPixels !== undefined) {
      cyRef.current?.resize()
      setTimeout(() => fitView(), 100)
    }
  }, [panelSize?.inPixels, fitView])

  return (
    <div className="flex h-full flex-col [--graph-node-bg:rgba(255,255,255,1)] [--graph-node-border:rgba(228,228,231,1)] [--graph-node-text:rgba(24,24,27,1)] dark:[--graph-node-bg:rgba(24,24,27,1)] dark:[--graph-node-border:rgba(63,63,70,1)] dark:[--graph-node-text:rgba(250,250,250,1)]">
      {/* Graph Header */}
      <header className="flex h-14 shrink-0 items-center justify-between border-border/40 border-b bg-background/80 px-3 backdrop-blur-md">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8 shrink-0"
            onClick={handleBack}
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <img
            src={PRODUCT_LOGO_URL}
            alt="BrowserOS"
            className="h-8 w-8 shrink-0"
          />
          {isEditingName ? (
            <input
              type="text"
              value={graphName}
              onChange={(e) => onGraphNameChange(e.target.value)}
              onBlur={() => setIsEditingName(false)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') setIsEditingName(false)
              }}
              // biome-ignore lint/a11y/noAutofocus: needed to autofocus field when edit mode is toggled
              autoFocus
              placeholder="Enter workflow name..."
              className="max-w-64 border-[var(--accent-orange)] border-b bg-transparent font-semibold text-sm outline-none placeholder:font-normal placeholder:text-muted-foreground/60"
            />
          ) : (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setIsEditingName(true)}
              className="group min-w-0 gap-2 px-2 py-1"
            >
              {graphName ? (
                <span className="truncate font-semibold text-sm">
                  {graphName}
                </span>
              ) : (
                <span className="text-muted-foreground/60 text-sm italic">
                  Untitled workflow
                </span>
              )}
              <Pencil className="h-3.5 w-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
            </Button>
          )}
        </div>

        {/* Control Buttons */}
        <div className="flex items-center gap-2">
          <Tooltip>
            <TooltipTrigger asChild>
              <span>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={onClickTest}
                  disabled={!canTest}
                >
                  <Play className="mr-1.5 h-4 w-4" />
                  Test Workflow
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>{getTestTooltip()}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <span>
                <Button
                  size="sm"
                  onClick={onClickSave}
                  disabled={!canSave}
                  className="bg-[var(--accent-orange)] shadow-lg shadow-orange-500/20 hover:bg-[var(--accent-orange-bright)] disabled:bg-[var(--accent-orange)]/50"
                >
                  <Save className="mr-1.5 h-4 w-4" />
                  {getSaveButtonLabel()}
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>{getSaveTooltip()}</TooltipContent>
          </Tooltip>
        </div>
      </header>

      {/* Graph Canvas */}
      <div className="relative min-h-0 flex-1 overflow-hidden [--dot-color:rgba(0,0,0,0.2)] dark:[--dot-color:rgba(255,255,255,0.15)]">
        <div
          ref={containerRef}
          className="h-full w-full bg-zinc-50 dark:bg-zinc-900"
          style={{
            backgroundImage:
              'radial-gradient(circle, var(--dot-color) 1.5px, transparent 1.5px)',
            backgroundSize: '20px 20px',
          }}
        />

        {/* Zoom Controls */}
        <div className="absolute bottom-4 left-4 z-10 flex flex-col gap-1 rounded-lg border-2 border-border bg-card p-1">
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={zoomIn}
            title="Zoom in"
          >
            <Plus className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={zoomOut}
            title="Zoom out"
          >
            <Minus className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={fitView}
            title="Fit view"
          >
            <Maximize className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  )
}
