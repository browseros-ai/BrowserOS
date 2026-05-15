import React, { useCallback, useMemo } from 'react';
import ReactFlow, {
  Node,
  Edge,
  Controls,
  Background,
  useNodesState,
  useEdgesState,
  addEdge,
  Connection,
  MarkerType,
} from 'reactflow';
import 'reactflow/dist/style.css';
import { motion } from 'framer-motion';
import { Sparkles, Zap, Lock } from 'lucide-react';

interface VisualFlowBuilderProps {
  initialDescription?: string;
  autoGenerate?: boolean;
  onWorkflowGenerated?: (workflow: any) => void;
  readOnly?: boolean;
}

interface FlowNode extends Node {
  data: {
    label: string;
    icon: string;
    type: 'trigger' | 'action' | 'condition';
  };
}

const defaultNodes: FlowNode[] = [
  {
    id: '1',
    type: 'default',
    position: { x: 100, y: 100 },
    data: { 
      label: '開始', 
      icon: '🚀',
      type: 'trigger'
    },
    className: 'node-trigger',
  },
];

const CustomNode = ({ data }: { data: any }) => (
  <motion.div
    initial={{ scale: 0.8, opacity: 0 }}
    animate={{ scale: 1, opacity: 1 }}
    className="px-4 py-3 bg-white dark:bg-gray-800 rounded-xl shadow-lg border-2 border-blue-200 dark:border-blue-700 min-w-[150px]"
  >
    <div className="flex items-center gap-3">
      <span className="text-2xl">{data.icon}</span>
      <div>
        <p className="font-medium text-gray-900 dark:text-white text-sm">
          {data.label}
        </p>
        <p className="text-xs text-gray-500 dark:text-gray-400 capitalize">
          {data.type}
        </p>
      </div>
    </div>
  </motion.div>
);

export function VisualFlowBuilder({
  initialDescription = '',
  autoGenerate = false,
  onWorkflowGenerated,
  readOnly = false,
}: VisualFlowBuilderProps) {
  const [nodes, setNodes, onNodesChange] = useNodesState(defaultNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);

  const onConnect = useCallback(
    (params: Connection) =>
      setEdges((eds) =>
        addEdge(
          {
            ...params,
            type: 'smoothstep',
            markerEnd: { type: MarkerType.ArrowClosed },
            animated: true,
            style: { stroke: '#3b82f6', strokeWidth: 2 },
          },
          eds
        )
      ),
    [setEdges]
  );

  // Auto-generate workflow from description
  React.useEffect(() => {
    if (autoGenerate && initialDescription) {
      generateWorkflowFromText(initialDescription);
    }
  }, [autoGenerate, initialDescription]);

  const generateWorkflowFromText = async (description: string) => {
    // Call MCP service to generate workflow
    try {
      const response = await fetch('/api/accessibility/generate-workflow', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ description }),
      });
      
      const result = await response.json();
      
      if (result.success && result.workflow) {
        setNodes(result.workflow.nodes || []);
        setEdges(result.workflow.edges || []);
        onWorkflowGenerated?.(result.workflow);
      }
    } catch (error) {
      console.error('Failed to generate workflow:', error);
    }
  };

  const nodeTypes = useMemo(() => ({ custom: CustomNode }), []);

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      className="w-full h-[600px] bg-gradient-to-br from-gray-50 to-gray-100 dark:from-gray-900 dark:to-gray-800 rounded-2xl overflow-hidden shadow-xl"
    >
      {/* Header */}
      <div className="absolute top-4 left-4 z-10 flex items-center gap-2 px-4 py-2 bg-white/90 dark:bg-gray-800/90 backdrop-blur-sm rounded-lg shadow-md">
        <Zap className="w-5 h-5 text-yellow-500" />
        <h3 className="font-semibold text-gray-900 dark:text-white">
          視覺化流程編排器
        </h3>
        {readOnly && (
          <Lock className="w-4 h-4 text-gray-400" />
        )}
      </div>

      {/* AI Generate Button */}
      {!readOnly && (
        <motion.button
          whileHover={{ scale: 1.05 }}
          whileTap={{ scale: 0.95 }}
          onClick={() => {
            const desc = prompt('請描述您想要的工作流程：\n例如：「當我收到郵件時，自動儲存附件到 Google Drive」');
            if (desc) generateWorkflowFromText(desc);
          }}
          className="absolute top-4 right-4 z-10 flex items-center gap-2 px-4 py-2 bg-gradient-to-r from-purple-500 to-pink-500 text-white rounded-lg shadow-lg hover:shadow-xl transition-shadow"
        >
          <Sparkles className="w-4 h-4" />
          <span className="text-sm font-medium">AI 生成</span>
        </motion.button>
      )}

      {/* React Flow Canvas */}
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={readOnly ? undefined : onNodesChange}
        onEdgesChange={readOnly ? undefined : onEdgesChange}
        onConnect={readOnly ? undefined : onConnect}
        nodeTypes={nodeTypes}
        fitView
        snapToGrid
        snapGrid={[15, 15]}
        defaultEdgeOptions={{
          type: 'smoothstep',
          markerEnd: { type: MarkerType.ArrowClosed },
          animated: true,
          style: { stroke: '#3b82f6', strokeWidth: 2 },
        }}
        className="bg-transparent"
        proOptions={{ hideAttribution: true }}
      >
        <Controls 
          className="bg-white dark:bg-gray-800 rounded-lg shadow-md"
          showInteractive={false}
        />
        <Background 
          color="#888" 
          gap={20} 
          size={1} 
          variant="dots"
        />
      </ReactFlow>

      {/* Help Text */}
      <div className="absolute bottom-4 left-4 z-10 px-4 py-2 bg-white/80 dark:bg-gray-800/80 backdrop-blur-sm rounded-lg text-xs text-gray-600 dark:text-gray-400">
        💡 提示：拖曳節點來調整位置，從連接點拖曳來建立連線
      </div>
    </motion.div>
  );
}

export default VisualFlowBuilder;
