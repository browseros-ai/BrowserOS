// Copyright 2026 The Chromium Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

/**
 * @fileoverview 'ghost-mode-workflow-editor' is a visual editor for Ghost Mode
 * generated workflows. Allows users to modify, reorder, and test workflow steps.
 */

import '../settings_shared.css.js';
import 'chrome://resources/cr_elements/cr_button/cr_button.js';
import 'chrome://resources/cr_elements/cr_icon/cr_icon.js';
import 'chrome://resources/cr_elements/cr_shared_style.css.js';

import {PolymerElement} from 'chrome://resources/polymer/v3_0/polymer/polymer_bundled.min.js';

import {getTemplate} from './workflow_editor.html.js';

export interface WorkflowStep {
  type: 'navigate' | 'click' | 'type' | 'scroll' | 'wait' | 'select';
  url?: string;
  selector?: string;
  value?: string;
  timeout?: number;
  scrollX?: number;
  scrollY?: number;
}

export interface Workflow {
  name: string;
  version: string;
  steps: WorkflowStep[];
  metadata: {
    createdAt: string;
    source: string;
    patternId?: string;
  };
}

export class GhostModeWorkflowEditorElement extends PolymerElement {
  static get is() {
    return 'ghost-mode-workflow-editor';
  }

  static get template() {
    return getTemplate();
  }

  static get properties() {
    return {
      /**
       * The workflow being edited
       */
      workflow: {
        type: Object,
        observer: 'onWorkflowChanged_',
      },
      
      /**
       * Workflow name for display
       */
      workflowName_: {
        type: String,
        value: 'New Workflow',
      },
      
      /**
       * Steps in the workflow
       */
      steps_: {
        type: Array,
        value: () => [],
      },
      
      /**
       * Currently dragged step index
       */
      draggedIndex_: {
        type: Number,
        value: -1,
      },
    };
  }

  declare workflow: Workflow | null;
  declare workflowName_: string;
  declare steps_: WorkflowStep[];
  declare draggedIndex_: number;

  private onWorkflowChanged_() {
    if (this.workflow) {
      this.workflowName_ = this.workflow.name;
      this.steps_ = [...this.workflow.steps];
    }
  }

  private getStepNumber_(index: number): number {
    return index + 1;
  }

  private getStepIcon_(type: string): string {
    switch (type) {
      case 'navigate': return '🌐';
      case 'click': return '👆';
      case 'type': return '⌨️';
      case 'scroll': return '📜';
      case 'wait': return '⏱️';
      case 'select': return '📋';
      default: return '•';
    }
  }

  private isNavigateStep_(step: WorkflowStep): boolean {
    return step.type === 'navigate';
  }

  private isClickOrTypeStep_(step: WorkflowStep): boolean {
    return step.type === 'click' || step.type === 'type' || step.type === 'select';
  }

  private isTypeStep_(step: WorkflowStep): boolean {
    return step.type === 'type';
  }

  private isWaitStep_(step: WorkflowStep): boolean {
    return step.type === 'wait';
  }

  // Drag and drop handlers
  private onDragStart_(e: DragEvent) {
    const target = e.target as HTMLElement;
    const stepItem = target.closest('.step-item');
    if (stepItem) {
      const items = Array.from(this.shadowRoot!.querySelectorAll('.step-item'));
      this.draggedIndex_ = items.indexOf(stepItem);
      stepItem.classList.add('dragging');
    }
  }

  private onDragEnd_(e: DragEvent) {
    const target = e.target as HTMLElement;
    const stepItem = target.closest('.step-item');
    if (stepItem) {
      stepItem.classList.remove('dragging');
    }
    this.draggedIndex_ = -1;
  }

  private onDragOver_(e: DragEvent) {
    e.preventDefault();
  }

  private onDrop_(e: DragEvent) {
    e.preventDefault();
    const target = e.target as HTMLElement;
    const stepItem = target.closest('.step-item');
    if (stepItem && this.draggedIndex_ >= 0) {
      const items = Array.from(this.shadowRoot!.querySelectorAll('.step-item'));
      const dropIndex = items.indexOf(stepItem);
      
      if (dropIndex !== this.draggedIndex_) {
        const movedStep = this.steps_[this.draggedIndex_];
        this.splice('steps_', this.draggedIndex_, 1);
        this.splice('steps_', dropIndex, 0, movedStep);
      }
    }
  }

  private onMoveUp_(e: Event) {
    const index = (e.model as any).index as number;
    if (index > 0) {
      const step = this.steps_[index];
      this.splice('steps_', index, 1);
      this.splice('steps_', index - 1, 0, step);
    }
  }

  private onMoveDown_(e: Event) {
    const index = (e.model as any).index as number;
    if (index < this.steps_.length - 1) {
      const step = this.steps_[index];
      this.splice('steps_', index, 1);
      this.splice('steps_', index + 1, 0, step);
    }
  }

  private onDeleteStep_(e: Event) {
    const index = (e.model as any).index as number;
    if (confirm('Delete this step?')) {
      this.splice('steps_', index, 1);
    }
  }

  private onFieldChange_(e: Event) {
    const input = e.target as HTMLInputElement;
    const stepItem = input.closest('.step-item');
    if (stepItem) {
      const items = Array.from(this.shadowRoot!.querySelectorAll('.step-item'));
      const index = items.indexOf(stepItem);
      const fieldName = input.closest('.step-field')?.querySelector('.step-field-label')?.textContent?.toLowerCase();
      
      if (index >= 0 && fieldName) {
        const step = {...this.steps_[index]};
        if (fieldName.includes('url')) step.url = input.value;
        if (fieldName.includes('selector')) step.selector = input.value;
        if (fieldName.includes('value')) step.value = input.value;
        if (fieldName.includes('wait')) step.timeout = parseInt(input.value, 10);
        
        this.set(`steps_.${index}`, step);
      }
    }
  }

  private showAddStepMenu_() {
    // In production, show a dropdown menu with step types
    const stepType = prompt('Enter step type: navigate, click, type, wait, scroll');
    if (stepType && ['navigate', 'click', 'type', 'wait', 'scroll', 'select'].includes(stepType)) {
      const newStep: WorkflowStep = {type: stepType as any};
      
      if (stepType === 'navigate') newStep.url = 'https://';
      if (stepType === 'click' || stepType === 'type') newStep.selector = '';
      if (stepType === 'type') newStep.value = '';
      if (stepType === 'wait') newStep.timeout = 1000;
      
      this.push('steps_', newStep);
    }
  }

  private onTestRun_() {
    this.dispatchEvent(new CustomEvent('test-workflow', {
      detail: {steps: this.steps_},
      bubbles: true,
      composed: true,
    }));
  }

  private onSave_() {
    const updatedWorkflow: Workflow = {
      name: this.workflowName_,
      version: '1.0.0',
      steps: this.steps_,
      metadata: {
        createdAt: new Date().toISOString(),
        source: 'ghost-mode-editor',
      },
    };
    
    this.dispatchEvent(new CustomEvent('save-workflow', {
      detail: {workflow: updatedWorkflow},
      bubbles: true,
      composed: true,
    }));
  }

  private onCancel_() {
    this.dispatchEvent(new CustomEvent('cancel-edit', {
      bubbles: true,
      composed: true,
    }));
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'ghost-mode-workflow-editor': GhostModeWorkflowEditorElement;
  }
}

customElements.define(
  GhostModeWorkflowEditorElement.is,
  GhostModeWorkflowEditorElement
);
