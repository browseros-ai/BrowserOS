diff --git a/chrome/browser/browseros/ghost_mode/ghost_executor.cc b/chrome/browser/browseros/ghost_mode/ghost_executor.cc
new file mode 100644
index 0000000000000..e1f2a3b4c5d6e
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_executor.cc
@@ -0,0 +1,303 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/ghost_executor.h"
+
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/task/sequenced_task_runner.h"
+#include "base/uuid.h"
+#include "chrome/browser/browseros/ghost_mode/workflow_generator.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/browser.h"
+#include "chrome/browser/ui/browser_list.h"
+#include "content/public/browser/web_contents.h"
+
+namespace browseros::ghost_mode {
+
+namespace {
+
+std::string GenerateExecutionId() {
+  return "exec_" + base::Uuid::GenerateRandomV4().AsLowercaseString();
+}
+
+}  // namespace
+
+GhostExecutor::GhostExecutor(Profile* profile) : profile_(profile) {
+  CHECK(profile_);
+}
+
+GhostExecutor::~GhostExecutor() {
+  // Cancel all active executions
+  for (auto& [id, state] : active_executions_) {
+    FinishExecution(state.get(), ExecutionStatus::kCancelled,
+                     "Executor shutting down");
+  }
+  active_executions_.clear();
+  
+  // Clear queue
+  while (!execution_queue_.empty()) {
+    execution_queue_.pop();
+  }
+}
+
+std::string GhostExecutor::ExecuteWorkflow(const base::Value::Dict& workflow,
+                                            const base::Value::Dict& parameters) {
+  std::string execution_id = GenerateExecutionId();
+
+  auto state = std::make_unique<ExecutionState>();
+  state->execution_id = execution_id;
+  state->workflow = workflow.Clone();
+  state->parameters = parameters.Clone();
+  state->result.workflow_id = *workflow.FindString("id");
+  state->result.execution_id = execution_id;
+  state->result.status = ExecutionStatus::kPending;
+
+  const std::string* name = workflow.FindString("name");
+  LOG(INFO) << "browseros: Queuing workflow execution: "
+            << (name ? *name : "unnamed") << " (" << execution_id << ")";
+
+  // Add to queue
+  execution_queue_.push(std::move(state));
+
+  // Process queue
+  ProcessQueue();
+
+  return execution_id;
+}
+
+std::string GhostExecutor::ExecutePattern(const ActionSequence& pattern,
+                                           const base::Value::Dict& parameters) {
+  // Convert pattern to workflow first
+  auto workflow = GetWorkflowGenerator().GenerateWorkflow(pattern);
+  return ExecuteWorkflow(workflow, parameters);
+}
+
+void GhostExecutor::PauseExecution(const std::string& execution_id) {
+  auto it = active_executions_.find(execution_id);
+  if (it == active_executions_.end()) {
+    LOG(WARNING) << "browseros: Cannot pause unknown execution: "
+                 << execution_id;
+    return;
+  }
+
+  auto* state = it->second.get();
+  if (state->result.status != ExecutionStatus::kRunning) {
+    return;
+  }
+
+  state->result.status = ExecutionStatus::kPaused;
+  VLOG(1) << "browseros: Paused execution: " << execution_id;
+
+  for (auto& observer : observers_) {
+    observer.OnExecutionPaused(execution_id);
+  }
+}
+
+void GhostExecutor::ResumeExecution(const std::string& execution_id) {
+  auto it = active_executions_.find(execution_id);
+  if (it == active_executions_.end()) {
+    return;
+  }
+
+  auto* state = it->second.get();
+  if (state->result.status != ExecutionStatus::kPaused) {
+    return;
+  }
+
+  state->result.status = ExecutionStatus::kRunning;
+  VLOG(1) << "browseros: Resumed execution: " << execution_id;
+
+  // Continue with next step
+  ExecuteStep(state);
+}
+
+void GhostExecutor::CancelExecution(const std::string& execution_id) {
+  auto it = active_executions_.find(execution_id);
+  if (it == active_executions_.end()) {
+    return;
+  }
+
+  FinishExecution(it->second.get(), ExecutionStatus::kCancelled,
+                   "Cancelled by user");
+}
+
+ExecutionStatus GhostExecutor::GetStatus(const std::string& execution_id) {
+  auto it = active_executions_.find(execution_id);
+  if (it != active_executions_.end()) {
+    return it->second->result.status;
+  }
+  return ExecutionStatus::kPending;
+}
+
+std::optional<ExecutionResult> GhostExecutor::GetResult(
+    const std::string& execution_id) {
+  auto it = active_executions_.find(execution_id);
+  if (it != active_executions_.end()) {
+    return it->second->result;
+  }
+  return std::nullopt;
+}
+
+void GhostExecutor::ClearQueue() {
+  while (!execution_queue_.empty()) {
+    execution_queue_.pop();
+  }
+  VLOG(1) << "browseros: Execution queue cleared";
+}
+
+void GhostExecutor::AddObserver(ExecutionObserver* observer) {
+  observers_.AddObserver(observer);
+}
+
+void GhostExecutor::RemoveObserver(ExecutionObserver* observer) {
+  observers_.RemoveObserver(observer);
+}
+
+void GhostExecutor::ProcessQueue() {
+  // Check if we can start more executions
+  if (static_cast<int>(active_executions_.size()) >= max_concurrent_) {
+    return;
+  }
+
+  if (execution_queue_.empty()) {
+    return;
+  }
+
+  // Move from queue to active
+  auto state = std::move(execution_queue_.front());
+  execution_queue_.pop();
+
+  std::string execution_id = state->execution_id;
+
+  // Get start URL from workflow
+  std::string start_url;
+  const base::Value::Dict* trigger = state->workflow.FindDict("trigger");
+  if (trigger) {
+    const std::string* url_pattern = trigger->FindString("url_pattern");
+    if (url_pattern) {
+      start_url = *url_pattern;
+    }
+  }
+
+  // Create execution context
+  state->web_contents = CreateExecutionContext(start_url);
+  if (!state->web_contents) {
+    FinishExecution(state.get(), ExecutionStatus::kFailed,
+                     "Failed to create execution context");
+    return;
+  }
+
+  // Start execution
+  state->result.status = ExecutionStatus::kRunning;
+  state->result.started_at = base::Time::Now();
+
+  active_executions_[execution_id] = std::move(state);
+
+  NotifyExecutionStarted(execution_id);
+
+  // Execute first step
+  ExecuteStep(active_executions_[execution_id].get());
+}
+
+void GhostExecutor::ExecuteStep(ExecutionState* state) {
+  if (!state || state->result.status != ExecutionStatus::kRunning) {
+    return;
+  }
+
+  const base::Value::List* steps = state->workflow.FindList("steps");
+  if (!steps) {
+    FinishExecution(state, ExecutionStatus::kFailed, "No steps in workflow");
+    return;
+  }
+
+  if (state->current_step_index >= static_cast<int>(steps->size())) {
+    // All steps completed
+    FinishExecution(state, ExecutionStatus::kCompleted);
+    return;
+  }
+
+  const base::Value::Dict* step = 
+      (*steps)[state->current_step_index].GetIfDict();
+  if (!step) {
+    FinishExecution(state, ExecutionStatus::kFailed, "Invalid step format");
+    return;
+  }
+
+  const std::string* step_id = step->FindString("id");
+  const std::string* action = step->FindString("action");
+
+  VLOG(1) << "browseros: Executing step " << (state->current_step_index + 1)
+          << "/" << steps->size() << ": " << (action ? *action : "unknown");
+
+  // TODO: Implement actual step execution via CDP/automation API
+  // For now, simulate successful execution
+  base::SequencedTaskRunner::GetCurrentDefault()->PostDelayedTask(
+      FROM_HERE,
+      base::BindOnce(&GhostExecutor::OnStepComplete,
+                     weak_factory_.GetWeakPtr(), state,
+                     StepResult{
+                         .step_id = step_id ? *step_id : "",
+                         .success = true,
+                         .error_message = "",
+                         .duration = base::Milliseconds(500),
+                     }),
+      base::Milliseconds(500));
+}
+
+void GhostExecutor::OnStepComplete(ExecutionState* state, StepResult result) {
+  if (!state || state->result.status != ExecutionStatus::kRunning) {
+    return;
+  }
+
+  state->result.step_results.push_back(result);
+  NotifyStepCompleted(state->execution_id, result);
+
+  if (!result.success) {
+    // Check error handling from step
+    const base::Value::List* steps = state->workflow.FindList("steps");
+    if (steps && state->current_step_index < static_cast<int>(steps->size())) {
+      const base::Value::Dict* step =
+          (*steps)[state->current_step_index].GetIfDict();
+      if (step) {
+        const base::Value::Dict* error_handling =
+            step->FindDict("error_handling");
+        if (error_handling) {
+          const std::string* on_error = error_handling->FindString("on_error");
+          if (on_error && *on_error == "continue") {
+            // Continue despite error
+            state->current_step_index++;
+            ExecuteStep(state);
+            return;
+          }
+        }
+      }
+    }
+
+    // Default: fail on error
+    FinishExecution(state, ExecutionStatus::kFailed, result.error_message);
+    return;
+  }
+
+  // Move to next step
+  state->current_step_index++;
+  ExecuteStep(state);
+}
+
+void GhostExecutor::FinishExecution(ExecutionState* state,
+                                     ExecutionStatus status,
+                                     const std::string& error) {
+  if (!state) {
+    return;
+  }
+
+  state->result.status = status;
+  state->result.completed_at = base::Time::Now();
+  state->result.error_message = error;
+
+  LOG(INFO) << "browseros: Execution finished: " << state->execution_id
+            << " (status: " << static_cast<int>(status) << ")";
+
+  NotifyExecutionFinished(state->result);
+
+  // Cleanup
+  if (state->web_contents) {
+    // Close the execution tab
+    // Note: In actual implementation, handle cleanup properly
+    state->web_contents = nullptr;
+  }
+
+  active_executions_.erase(state->execution_id);
+
+  // Process more from queue
+  ProcessQueue();
+}
+
+content::WebContents* GhostExecutor::CreateExecutionContext(
+    const std::string& start_url) {
+  // Get a browser for this profile
+  Browser* browser = nullptr;
+  for (Browser* b : *BrowserList::GetInstance()) {
+    if (b->profile() == profile_) {
+      browser = b;
+      break;
+    }
+  }
+
+  if (!browser) {
+    LOG(WARNING) << "browseros: No browser found for profile";
+    return nullptr;
+  }
+
+  // TODO: Create actual background tab or headless context
+  // For now, return nullptr to indicate we need the full implementation
+  VLOG(1) << "browseros: Would create execution context for: " << start_url;
+  
+  // Placeholder: In real implementation, create a background WebContents
+  // or use headless mode for execution
+  return nullptr;
+}
+
+void GhostExecutor::NotifyExecutionStarted(const std::string& execution_id) {
+  for (auto& observer : observers_) {
+    observer.OnExecutionStarted(execution_id);
+  }
+}
+
+void GhostExecutor::NotifyStepCompleted(const std::string& execution_id,
+                                         const StepResult& result) {
+  for (auto& observer : observers_) {
+    observer.OnStepCompleted(execution_id, result);
+  }
+}
+
+void GhostExecutor::NotifyExecutionFinished(const ExecutionResult& result) {
+  for (auto& observer : observers_) {
+    observer.OnExecutionFinished(result);
+  }
+}
+
+}  // namespace browseros::ghost_mode
