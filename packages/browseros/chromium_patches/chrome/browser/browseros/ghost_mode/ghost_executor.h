diff --git a/chrome/browser/browseros/ghost_mode/ghost_executor.h b/chrome/browser/browseros/ghost_mode/ghost_executor.h
new file mode 100644
index 0000000000000..d0e1f2a3b4c5d
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_executor.h
@@ -0,0 +1,155 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_EXECUTOR_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_EXECUTOR_H_
+
+#include <memory>
+#include <queue>
+#include <string>
+#include <vector>
+
+#include "base/callback_forward.h"
+#include "base/memory/raw_ptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/observer_list.h"
+#include "base/values.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+
+class Profile;
+
+namespace content {
+class WebContents;
+}  // namespace content
+
+namespace browseros::ghost_mode {
+
+// Execution status for a workflow run
+enum class ExecutionStatus {
+  kPending,      // Queued for execution
+  kRunning,      // Currently executing
+  kPaused,       // Paused by user
+  kCompleted,    // Successfully finished
+  kFailed,       // Error occurred
+  kCancelled,    // Cancelled by user
+};
+
+// Result of a single step execution
+struct StepResult {
+  std::string step_id;
+  bool success = false;
+  std::string error_message;
+  base::TimeDelta duration;
+  base::Value::Dict extracted_data;  // If step extracted any data
+};
+
+// Result of a complete workflow execution
+struct ExecutionResult {
+  std::string workflow_id;
+  std::string execution_id;
+  ExecutionStatus status = ExecutionStatus::kPending;
+  std::vector<StepResult> step_results;
+  base::Time started_at;
+  base::Time completed_at;
+  std::string error_message;
+  
+  bool IsSuccess() const { return status == ExecutionStatus::kCompleted; }
+  base::TimeDelta GetDuration() const { return completed_at - started_at; }
+};
+
+// Observer for execution events
+class ExecutionObserver {
+ public:
+  virtual ~ExecutionObserver() = default;
+  
+  // Called when execution starts
+  virtual void OnExecutionStarted(const std::string& execution_id) {}
+  
+  // Called when a step completes
+  virtual void OnStepCompleted(const std::string& execution_id,
+                               const StepResult& result) {}
+  
+  // Called when execution finishes (success or failure)
+  virtual void OnExecutionFinished(const ExecutionResult& result) {}
+  
+  // Called when execution is paused
+  virtual void OnExecutionPaused(const std::string& execution_id) {}
+  
+  // Called when user interaction is needed
+  virtual void OnInteractionRequired(const std::string& execution_id,
+                                      const std::string& message) {}
+};
+
+// GhostExecutor runs workflows in background tabs or headlessly.
+// It handles step-by-step execution, error recovery, and user interaction.
+class GhostExecutor {
+ public:
+  explicit GhostExecutor(Profile* profile);
+  ~GhostExecutor();
+
+  // Execute a workflow (returns execution ID)
+  std::string ExecuteWorkflow(const base::Value::Dict& workflow,
+                               const base::Value::Dict& parameters);
+
+  // Execute from an ActionSequence directly
+  std::string ExecutePattern(const ActionSequence& pattern,
+                              const base::Value::Dict& parameters);
+
+  // Execution control
+  void PauseExecution(const std::string& execution_id);
+  void ResumeExecution(const std::string& execution_id);
+  void CancelExecution(const std::string& execution_id);
+
+  // Get execution status
+  ExecutionStatus GetStatus(const std::string& execution_id);
+  std::optional<ExecutionResult> GetResult(const std::string& execution_id);
+
+  // Queue management
+  int GetQueueSize() const { return static_cast<int>(execution_queue_.size()); }
+  void ClearQueue();
+
+  // Observer management
+  void AddObserver(ExecutionObserver* observer);
+  void RemoveObserver(ExecutionObserver* observer);
+
+  // Configuration
+  void SetMaxConcurrentExecutions(int max) { max_concurrent_ = max; }
+  void SetStepTimeout(base::TimeDelta timeout) { step_timeout_ = timeout; }
+  void SetUseBackgroundTab(bool use_background) {
+    use_background_tab_ = use_background;
+  }
+
+ private:
+  // Internal execution state
+  struct ExecutionState {
+    std::string execution_id;
+    base::Value::Dict workflow;
+    base::Value::Dict parameters;
+    ExecutionResult result;
+    int current_step_index = 0;
+    raw_ptr<content::WebContents> web_contents = nullptr;
+  };
+
+  // Start next execution from queue
+  void ProcessQueue();
+
+  // Execute a single step
+  void ExecuteStep(ExecutionState* state);
+
+  // Handle step completion
+  void OnStepComplete(ExecutionState* state, StepResult result);
+
+  // Finish execution
+  void FinishExecution(ExecutionState* state, ExecutionStatus status,
+                        const std::string& error = "");
+
+  // Create execution WebContents
+  content::WebContents* CreateExecutionContext(const std::string& start_url);
+
+  // Notify observers
+  void NotifyExecutionStarted(const std::string& execution_id);
+  void NotifyStepCompleted(const std::string& execution_id,
+                            const StepResult& result);
+  void NotifyExecutionFinished(const ExecutionResult& result);
+
+  // Dependencies
+  raw_ptr<Profile> profile_;
+
+  // Active executions
+  std::map<std::string, std::unique_ptr<ExecutionState>> active_executions_;
+
+  // Execution queue
+  std::queue<std::unique_ptr<ExecutionState>> execution_queue_;
+
+  // Configuration
+  int max_concurrent_ = 3;
+  base::TimeDelta step_timeout_ = base::Seconds(30);
+  bool use_background_tab_ = true;
+
+  // Observers
+  base::ObserverList<ExecutionObserver> observers_;
+
+  // Weak pointer factory
+  base::WeakPtrFactory<GhostExecutor> weak_factory_{this};
+};
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_EXECUTOR_H_
