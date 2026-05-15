/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { MCPTool } from '@browseros/agent-sdk';

interface N8NWorkflow {
  id: string;
  name: string;
  active: boolean;
  tags?: string[];
}

interface N8NExecutionResult {
  success: boolean;
  data?: any;
  error?: string;
}

export class N8NProvider {
  private baseUrl: string;
  private apiKey?: string;

  constructor(baseUrl?: string, apiKey?: string) {
    this.baseUrl = baseUrl || process.env.N8N_BASE_URL || 'http://localhost:5678';
    this.apiKey = apiKey || process.env.N8N_API_KEY;
  }

  /**
   * 獲取所有可用的 n8n 工作流
   */
  async listWorkflows(): Promise<N8NWorkflow[]> {
    try {
      const response = await fetch(`${this.baseUrl}/api/v1/workflows`, {
        headers: this.apiKey ? { 'X-N8N-API-KEY': this.apiKey } : {},
      });
      
      if (!response.ok) {
        console.warn('n8n API not available, returning empty workflow list');
        return [];
      }

      const data = await response.json();
      return data.data || [];
    } catch (error) {
      console.warn('Failed to fetch n8n workflows:', error);
      return [];
    }
  }

  /**
   * 執行指定的 n8n 工作流
   */
  async executeWorkflow(
    workflowId: string,
    inputData?: Record<string, any>
  ): Promise<N8NExecutionResult> {
    try {
      const response = await fetch(
        `${this.baseUrl}/api/v1/workflows/${workflowId}/execute`,
        {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            ...(this.apiKey ? { 'X-N8N-API-KEY': this.apiKey } : {}),
          },
          body: JSON.stringify({ data: inputData }),
        }
      );

      if (!response.ok) {
        throw new Error(`n8n execution failed: ${response.statusText}`);
      }

      const result = await response.json();
      return {
        success: true,
        data: result,
      };
    } catch (error: any) {
      return {
        success: false,
        error: error.message,
      };
    }
  }

  /**
   * 根據自然語言描述自動匹配並執行工作流
   */
  async autoExecuteFromDescription(
    description: string,
    context?: Record<string, any>
  ): Promise<N8NExecutionResult> {
    const workflows = await this.listWorkflows();
    
    // Simple keyword matching (can be enhanced with AI)
    const matchedWorkflow = workflows.find(wf => {
      const searchTerms = [wf.name, ...(wf.tags || [])].join(' ').toLowerCase();
      return description.toLowerCase().split(' ').some(term => 
        searchTerms.includes(term)
      );
    });

    if (!matchedWorkflow) {
      return {
        success: false,
        error: `No matching workflow found for: ${description}`,
      };
    }

    return this.executeWorkflow(matchedWorkflow.id, context);
  }
}

/**
 * 創建 n8n MCP 工具集
 */
export function createN8NTools(provider: N8NProvider): MCPTool[] {
  return [
    {
      name: 'n8n_list_workflows',
      description: '列出所有可用的 n8n 自動化工作流',
      inputSchema: {
        type: 'object',
        properties: {
          filter: {
            type: 'string',
            description: '可選的過濾器（按名稱或標籤）',
          },
        },
      },
      execute: async (params: { filter?: string }) => {
        const workflows = await provider.listWorkflows();
        const filtered = params.filter
          ? workflows.filter(wf => 
              wf.name.toLowerCase().includes(params.filter!.toLowerCase()) ||
              wf.tags?.some(tag => tag.toLowerCase().includes(params.filter!.toLowerCase()))
            )
          : workflows;
        
        return {
          count: filtered.length,
          workflows: filtered.map(wf => ({
            id: wf.id,
            name: wf.name,
            active: wf.active,
            tags: wf.tags,
          })),
        };
      },
    },

    {
      name: 'n8n_execute_workflow',
      description: '執行指定的 n8n 自動化工作流',
      inputSchema: {
        type: 'object',
        properties: {
          workflowId: {
            type: 'string',
            description: '工作流 ID',
          },
          workflowName: {
            type: 'string',
            description: '工作流名稱（如果不知道 ID）',
          },
          inputData: {
            type: 'object',
            description: '傳遞給工作流的輸入數據',
          },
        },
        required: [],
      },
      execute: async (params: { 
        workflowId?: string; 
        workflowName?: string; 
        inputData?: Record<string, any> 
      }) => {
        let workflowId = params.workflowId;
        
        if (!workflowId && params.workflowName) {
          const workflows = await provider.listWorkflows();
          const matched = workflows.find(wf => 
            wf.name.toLowerCase() === params.workflowName!.toLowerCase()
          );
          if (matched) {
            workflowId = matched.id;
          } else {
            return {
              success: false,
              error: `Workflow "${params.workflowName}" not found`,
            };
          }
        }

        if (!workflowId) {
          return {
            success: false,
            error: 'Either workflowId or workflowName must be provided',
          };
        }

        return provider.executeWorkflow(workflowId, params.inputData);
      },
    },

    {
      name: 'n8n_auto_execute',
      description: '根據自然語言描述自動匹配並執行 n8n 工作流',
      inputSchema: {
        type: 'object',
        properties: {
          description: {
            type: 'string',
            description: '想要執行的自動化任務描述（自然語言）',
          },
          context: {
            type: 'object',
            description: '任務相關的上下文數據',
          },
        },
        required: ['description'],
      },
      execute: async (params: { 
        description: string; 
        context?: Record<string, any> 
      }) => {
        return provider.autoExecuteFromDescription(params.description, params.context);
      },
    },
  ];
}
