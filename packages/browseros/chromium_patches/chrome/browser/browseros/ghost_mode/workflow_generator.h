diff --git a/chrome/browser/browseros/ghost_mode/workflow_generator.h b/chrome/browser/browseros/ghost_mode/workflow_generator.h
new file mode 100644
index 0000000000000..b8c9d0e1f2a3b
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/workflow_generator.h
@@ -0,0 +1,127 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_WORKFLOW_GENERATOR_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_WORKFLOW_GENERATOR_H_
+
+#include <string>
+#include <vector>
+
+#include "base/values.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+
+namespace browseros::ghost_mode {
+
+// WorkflowGenerator converts detected ActionSequences into BrowserOS
+// Workflow format that can be saved and executed.
+//
+// BrowserOS Workflow Format:
+// {
+//   "id": "uuid",
+//   "name": "Workflow Name",
+//   "description": "What this workflow does",
+//   "trigger": { ... },  // How/when to run
+//   "steps": [ ... ],    // Array of action steps
+//   "parameters": [ ... ], // User-configurable inputs
+//   "metadata": { ... }  // Additional info
+// }
+class WorkflowGenerator {
+ public:
+  WorkflowGenerator();
+  ~WorkflowGenerator();
+
+  // Convert an ActionSequence to a Workflow JSON
+  base::Value::Dict GenerateWorkflow(const ActionSequence& pattern);
+
+  // Generate workflow with custom name and description
+  base::Value::Dict GenerateWorkflow(const ActionSequence& pattern,
+                                      const std::string& name,
+                                      const std::string& description);
+
+  // Convert a single RecordedAction to a workflow step
+  base::Value::Dict ActionToStep(const RecordedAction& action,
+                                  int step_index);
+
+  // Generate parameters from parameterizable actions
+  std::vector<base::Value::Dict> ExtractParameters(
+      const ActionSequence& pattern);
+
+  // Generate trigger configuration
+  base::Value::Dict GenerateTrigger(const ActionSequence& pattern);
+
+  // Validate generated workflow
+  bool ValidateWorkflow(const base::Value::Dict& workflow);
+
+  // Export workflow to JSON string
+  std::string ExportToJson(const base::Value::Dict& workflow);
+
+  // Import workflow from JSON string
+  std::optional<base::Value::Dict> ImportFromJson(const std::string& json);
+
+ private:
+  // Convert ActionType to workflow action type string
+  std::string GetWorkflowActionType(ActionType type);
+
+  // Generate a human-readable step description
+  std::string GenerateStepDescription(const RecordedAction& action);
+
+  // Select best selector strategy for an action
+  base::Value::Dict GenerateSelector(const RecordedAction& action);
+
+  // Generate wait/timing hints for a step
+  base::Value::Dict GenerateTimingHints(const RecordedAction& action,
+                                         const RecordedAction* previous);
+
+  // Workflow ID counter for uniqueness
+  int workflow_counter_ = 0;
+};
+
+// TriggerType defines when a workflow should be activated
+enum class TriggerType {
+  kManual,      // User manually triggers
+  kUrlMatch,    // When visiting matching URL
+  kScheduled,   // At scheduled times
+  kContextual,  // Based on page content
+};
+
+// ParameterType for workflow input parameters
+enum class ParameterType {
+  kText,       // Free text input
+  kNumber,     // Numeric input
+  kSelect,     // Selection from options
+  kBoolean,    // Yes/no toggle
+  kFile,       // File selection
+  kPassword,   // Secure text input
+};
+
+// Singleton accessor
+WorkflowGenerator& GetWorkflowGenerator();
+
+// Convenience functions
+
+// Quick conversion from pattern to workflow JSON string
+std::string PatternToWorkflowJson(const ActionSequence& pattern);
+
+// Generate a unique workflow ID
+std::string GenerateWorkflowId();
+
+// Validate a workflow step
+bool IsValidWorkflowStep(const base::Value::Dict& step);
+
+// Step action constants (match BrowserOS workflow format)
+namespace workflow_actions {
+constexpr char kClick[] = "click";
+constexpr char kType[] = "type";
+constexpr char kNavigate[] = "navigate";
+constexpr char kWait[] = "wait";
+constexpr char kScroll[] = "scroll";
+constexpr char kSelect[] = "select";
+constexpr char kSubmit[] = "submit";
+constexpr char kKeyPress[] = "keypress";
+constexpr char kHover[] = "hover";
+constexpr char kDragDrop[] = "dragdrop";
+constexpr char kScreenshot[] = "screenshot";
+constexpr char kExtract[] = "extract";
+constexpr char kAssert[] = "assert";
+}  // namespace workflow_actions
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_WORKFLOW_GENERATOR_H_
