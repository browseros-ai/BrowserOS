import React, { useState, useEffect } from 'react';
import { motion } from 'framer-motion';

interface Workflow {
  id: string;
  name: string;
  active: boolean;
  tags?: string[];
}

export const N8NPanel: React.FC = () => {
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedWorkflow, setSelectedWorkflow] = useState<string | null>(null);
  const [executionResult, setExecutionResult] = useState<any>(null);

  // Load workflows on mount
  useEffect(() => {
    loadWorkflows();
  }, []);

  const loadWorkflows = async () => {
    try {
      // In real implementation, this would call the MCP tool
      // const result = await window.mcpClient.callTool('n8n_list_workflows', {});
      // setWorkflows(result.workflows || []);
      
      // Mock data for demonstration
      setWorkflows([
        { id: '1', name: 'Lead Enrichment', active: true, tags: ['crm', 'linkedin'] },
        { id: '2', name: 'Price Monitor', active: true, tags: ['ecommerce', 'alert'] },
        { id: '3', name: 'Form Auto-fill', active: false, tags: ['automation', 'forms'] },
      ]);
    } catch (error) {
      console.error('Failed to load workflows:', error);
    } finally {
      setLoading(false);
    }
  };

  const executeWorkflow = async (workflowId: string) => {
    setSelectedWorkflow(workflowId);
    setExecutionResult({ status: 'running' });

    try {
      // In real implementation, this would call the MCP tool
      // const result = await window.mcpClient.callTool('n8n_execute_workflow', { workflowId });
      
      // Mock execution
      await new Promise(resolve => setTimeout(resolve, 2000));
      setExecutionResult({ 
        status: 'success', 
        message: `工作流 "${workflowId}" 執行成功`,
        data: { items: 42, duration: '1.2s' }
      });
    } catch (error: any) {
      setExecutionResult({ 
        status: 'error', 
        message: error.message 
      });
    }
  };

  return (
    <div className="p-4 bg-white dark:bg-gray-900 rounded-lg shadow-lg">
      <h2 className="text-xl font-bold mb-4 text-gray-900 dark:text-white">
        🔄 n8n 自動化工作流
      </h2>

      {/* Workflow List */}
      <div className="space-y-2 mb-6">
        {loading ? (
          <div className="text-center py-4 text-gray-500">載入中...</div>
        ) : workflows.length === 0 ? (
          <div className="text-center py-4 text-gray-500">
            暫無可用工作流，請先在 n8n 中建立
          </div>
        ) : (
          workflows.map((wf) => (
            <motion.div
              key={wf.id}
              initial={{ opacity: 0, x: -20 }}
              animate={{ opacity: 1, x: 0 }}
              className={`
                p-3 rounded-lg border cursor-pointer transition-all
                ${selectedWorkflow === wf.id 
                  ? 'border-blue-500 bg-blue-50 dark:bg-blue-900/20' 
                  : 'border-gray-200 dark:border-gray-700 hover:border-blue-300'}
              `}
              onClick={() => executeWorkflow(wf.id)}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <span className="text-xl">⚡</span>
                  <div>
                    <div className="font-medium text-gray-900 dark:text-white">
                      {wf.name}
                    </div>
                    {wf.tags && (
                      <div className="flex gap-1 mt-1">
                        {wf.tags.map(tag => (
                          <span
                            key={tag}
                            className="text-xs px-2 py-0.5 bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400 rounded"
                          >
                            #{tag}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
                <span
                  className={`px-2 py-1 text-xs rounded ${
                    wf.active
                      ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                      : 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400'
                  }`}
                >
                  {wf.active ? '✓ 啟用' : '✗ 停用'}
                </span>
              </div>
            </motion.div>
          ))
        )}
      </div>

      {/* Execution Result */}
      {executionResult && (
        <motion.div
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          className={`
            p-4 rounded-lg border
            ${executionResult.status === 'success'
              ? 'border-green-300 bg-green-50 dark:bg-green-900/20'
              : executionResult.status === 'error'
              ? 'border-red-300 bg-red-50 dark:bg-red-900/20'
              : 'border-blue-300 bg-blue-50 dark:bg-blue-900/20'}
          `}
        >
          <div className="flex items-center gap-2 mb-2">
            {executionResult.status === 'running' && <span>⏳</span>}
            {executionResult.status === 'success' && <span>✅</span>}
            {executionResult.status === 'error' && <span>❌</span>}
            <span className="font-medium capitalize">
              {executionResult.status === 'running' && '執行中...'}
              {executionResult.status === 'success' && '執行成功'}
              {executionResult.status === 'error' && '執行失敗'}
            </span>
          </div>
          
          {executionResult.message && (
            <div className="text-sm text-gray-700 dark:text-gray-300">
              {executionResult.message}
            </div>
          )}
          
          {executionResult.data && (
            <pre className="mt-2 p-2 bg-white dark:bg-gray-800 rounded text-xs overflow-auto">
              {JSON.stringify(executionResult.data, null, 2)}
            </pre>
          )}
        </motion.div>
      )}

      {/* Open n8n Button */}
      <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
        <button
          onClick={() => window.open('http://localhost:5678', '_blank')}
          className="w-full py-2 px-4 bg-orange-500 hover:bg-orange-600 text-white rounded-lg transition-colors flex items-center justify-center gap-2"
        >
          <span>🔧</span>
          開啟 n8n 編輯器
        </button>
      </div>
    </div>
  );
};
