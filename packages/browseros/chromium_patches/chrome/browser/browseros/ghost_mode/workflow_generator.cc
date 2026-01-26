diff --git a/chrome/browser/browseros/ghost_mode/workflow_generator.cc b/chrome/browser/browseros/ghost_mode/workflow_generator.cc
new file mode 100644
index 0000000000000..c9d0e1f2a3b4c
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/workflow_generator.cc
@@ -0,0 +1,318 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/workflow_generator.h"
+
+#include "base/json/json_reader.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/no_destructor.h"
+#include "base/strings/string_util.h"
+#include "base/uuid.h"
+
+namespace browseros::ghost_mode {
+
+WorkflowGenerator::WorkflowGenerator() = default;
+WorkflowGenerator::~WorkflowGenerator() = default;
+
+base::Value::Dict WorkflowGenerator::GenerateWorkflow(
+    const ActionSequence& pattern) {
+  return GenerateWorkflow(pattern, pattern.name, pattern.description);
+}
+
+base::Value::Dict WorkflowGenerator::GenerateWorkflow(
+    const ActionSequence& pattern,
+    const std::string& name,
+    const std::string& description) {
+  base::Value::Dict workflow;
+
+  // Metadata
+  workflow.Set("id", GenerateWorkflowId());
+  workflow.Set("name", name);
+  workflow.Set("description", description.empty()
+                                   ? "Auto-generated from browsing pattern"
+                                   : description);
+  workflow.Set("version", "1.0");
+  workflow.Set("created_at", base::Time::Now().InMillisecondsSinceUnixEpoch());
+
+  // Source info (for debugging/attribution)
+  base::Value::Dict source;
+  source.Set("type", "ghost_mode");
+  source.Set("pattern_id", pattern.id);
+  source.Set("pattern_hash", pattern.pattern_hash);
+  source.Set("occurrence_count", pattern.occurrence_count);
+  source.Set("confidence_score", pattern.confidence_score);
+  workflow.Set("source", std::move(source));
+
+  // Trigger configuration
+  workflow.Set("trigger", GenerateTrigger(pattern));
+
+  // Convert actions to steps
+  base::Value::List steps;
+  for (size_t i = 0; i < pattern.actions.size(); ++i) {
+    const RecordedAction* previous = (i > 0) ? &pattern.actions[i - 1] : nullptr;
+    base::Value::Dict step = ActionToStep(pattern.actions[i], static_cast<int>(i));
+    
+    // Add timing hints based on previous action
+    if (previous) {
+      step.Set("timing", GenerateTimingHints(pattern.actions[i], previous));
+    }
+    
+    steps.Append(std::move(step));
+  }
+  workflow.Set("steps", std::move(steps));
+
+  // Extract parameters for user customization
+  auto params = ExtractParameters(pattern);
+  if (!params.empty()) {
+    base::Value::List params_list;
+    for (auto& param : params) {
+      params_list.Append(std::move(param));
+    }
+    workflow.Set("parameters", std::move(params_list));
+  }
+
+  // Additional metadata
+  base::Value::Dict metadata;
+  metadata.Set("url_pattern", pattern.url_pattern);
+  metadata.Set("step_count", static_cast<int>(pattern.actions.size()));
+  metadata.Set("auto_generated", true);
+  workflow.Set("metadata", std::move(metadata));
+
+  VLOG(1) << "browseros: Generated workflow with " << pattern.actions.size()
+          << " steps";
+
+  return workflow;
+}
+
+base::Value::Dict WorkflowGenerator::ActionToStep(const RecordedAction& action,
+                                                    int step_index) {
+  base::Value::Dict step;
+
+  step.Set("id", "step_" + base::NumberToString(step_index + 1));
+  step.Set("action", GetWorkflowActionType(action.type));
+  step.Set("description", GenerateStepDescription(action));
+
+  // Target element (selector)
+  step.Set("selector", GenerateSelector(action));
+
+  // Action-specific data
+  base::Value::Dict data;
+  
+  switch (action.type) {
+    case ActionType::kClick:
+      // Click typically doesn't need extra data
+      break;
+
+    case ActionType::kType:
+      if (action.is_parameterizable) {
+        // Reference a parameter instead of hardcoded value
+        data.Set("value_param", "param_" + base::NumberToString(step_index));
+        data.Set("clear_before", true);
+      } else {
+        data.Set("value", action.value);
+      }
+      break;
+
+    case ActionType::kNavigate:
+      data.Set("url", action.url.spec());
+      break;
+
+    case ActionType::kScroll:
+      // Scroll direction/amount from metadata
+      if (action.metadata.contains("scroll_y")) {
+        data.Set("scroll_y", action.metadata.FindDouble("scroll_y").value_or(0));
+      }
+      break;
+
+    case ActionType::kSelect:
+      data.Set("value", action.value);
+      break;
+
+    case ActionType::kSubmit:
+      // Submit usually just targets the form
+      break;
+
+    case ActionType::kKeyPress:
+      data.Set("key", action.value);
+      break;
+
+    case ActionType::kHover:
+      data.Set("duration_ms", 500);  // Default hover time
+      break;
+
+    case ActionType::kDragDrop:
+      // Drag target from metadata
+      if (action.metadata.contains("drop_selector")) {
+        data.Set("drop_selector",
+                 *action.metadata.FindString("drop_selector"));
+      }
+      break;
+  }
+
+  if (!data.empty()) {
+    step.Set("data", std::move(data));
+  }
+
+  // Error handling
+  base::Value::Dict error_handling;
+  error_handling.Set("on_error", "continue");  // or "stop", "retry"
+  error_handling.Set("retry_count", 2);
+  error_handling.Set("timeout_ms", 10000);
+  step.Set("error_handling", std::move(error_handling));
+
+  return step;
+}
+
+std::vector<base::Value::Dict> WorkflowGenerator::ExtractParameters(
+    const ActionSequence& pattern) {
+  std::vector<base::Value::Dict> params;
+  int param_index = 0;
+
+  for (size_t i = 0; i < pattern.actions.size(); ++i) {
+    const auto& action = pattern.actions[i];
+
+    if (action.is_parameterizable && action.type == ActionType::kType) {
+      base::Value::Dict param;
+      param.Set("id", "param_" + base::NumberToString(i));
+      param.Set("name", action.element_text.empty()
+                            ? ("Input " + base::NumberToString(++param_index))
+                            : action.element_text);
+      param.Set("type", "text");
+      param.Set("required", true);
+      param.Set("description", "Value for " + action.element_text);
+
+      // Default value (if not sensitive)
+      if (!action.value.empty()) {
+        param.Set("default", action.value);
+      }
+
+      params.push_back(std::move(param));
+    }
+  }
+
+  return params;
+}
+
+base::Value::Dict WorkflowGenerator::GenerateTrigger(
+    const ActionSequence& pattern) {
+  base::Value::Dict trigger;
+
+  // Default to URL match trigger
+  trigger.Set("type", "url_match");
+  trigger.Set("url_pattern", pattern.url_pattern);
+  trigger.Set("auto_run", false);  // User must confirm first time
+
+  // Alternative triggers
+  base::Value::List alternatives;
+  
+  base::Value::Dict manual_trigger;
+  manual_trigger.Set("type", "manual");
+  manual_trigger.Set("keyboard_shortcut", "");  // User can configure
+  alternatives.Append(std::move(manual_trigger));
+
+  trigger.Set("alternatives", std::move(alternatives));
+
+  return trigger;
+}
+
+bool WorkflowGenerator::ValidateWorkflow(const base::Value::Dict& workflow) {
+  // Required fields
+  if (!workflow.contains("id") || !workflow.contains("name") ||
+      !workflow.contains("steps")) {
+    return false;
+  }
+
+  // Steps must be a non-empty list
+  const base::Value::List* steps = workflow.FindList("steps");
+  if (!steps || steps->empty()) {
+    return false;
+  }
+
+  // Validate each step
+  for (const auto& step : *steps) {
+    if (!step.is_dict() || !IsValidWorkflowStep(step.GetDict())) {
+      return false;
+    }
+  }
+
+  return true;
+}
+
+std::string WorkflowGenerator::ExportToJson(const base::Value::Dict& workflow) {
+  std::string json;
+  base::JSONWriter::WriteWithOptions(
+      workflow, base::JSONWriter::OPTIONS_PRETTY_PRINT, &json);
+  return json;
+}
+
+std::optional<base::Value::Dict> WorkflowGenerator::ImportFromJson(
+    const std::string& json) {
+  auto value = base::JSONReader::Read(json);
+  if (!value || !value->is_dict()) {
+    return std::nullopt;
+  }
+  return std::move(value->GetDict());
+}
+
+std::string WorkflowGenerator::GetWorkflowActionType(ActionType type) {
+  switch (type) {
+    case ActionType::kClick:
+      return workflow_actions::kClick;
+    case ActionType::kType:
+      return workflow_actions::kType;
+    case ActionType::kNavigate:
+      return workflow_actions::kNavigate;
+    case ActionType::kScroll:
+      return workflow_actions::kScroll;
+    case ActionType::kSelect:
+      return workflow_actions::kSelect;
+    case ActionType::kSubmit:
+      return workflow_actions::kSubmit;
+    case ActionType::kKeyPress:
+      return workflow_actions::kKeyPress;
+    case ActionType::kHover:
+      return workflow_actions::kHover;
+    case ActionType::kDragDrop:
+      return workflow_actions::kDragDrop;
+  }
+  return "unknown";
+}
+
+std::string WorkflowGenerator::GenerateStepDescription(
+    const RecordedAction& action) {
+  auto summary = ActionSequence().GetActionSummary();
+  // Create temp sequence with one action to get summary
+  ActionSequence temp;
+  temp.actions.push_back(action);
+  auto summaries = temp.GetActionSummary();
+  return summaries.empty() ? "Perform action" : summaries[0];
+}
+
+base::Value::Dict WorkflowGenerator::GenerateSelector(
+    const RecordedAction& action) {
+  base::Value::Dict selector;
+
+  if (!action.selectors.empty()) {
+    // Use primary selector
+    selector.Set("css", action.selectors[0]);
+
+    // Add fallbacks
+    if (action.selectors.size() > 1) {
+      base::Value::List fallbacks;
+      for (size_t i = 1; i < action.selectors.size(); ++i) {
+        fallbacks.Append(action.selectors[i]);
+      }
+      selector.Set("fallbacks", std::move(fallbacks));
+    }
+  }
+
+  // Text-based fallback
+  if (!action.element_text.empty()) {
+    selector.Set("text", action.element_text);
+  }
+
+  return selector;
+}
+
+base::Value::Dict WorkflowGenerator::GenerateTimingHints(
+    const RecordedAction& action,
+    const RecordedAction* previous) {
+  base::Value::Dict timing;
+
+  // Use observed timing from recording
+  if (!action.time_since_previous.is_zero()) {
+    int delay_ms = static_cast<int>(action.time_since_previous.InMilliseconds());
+    
+    // Add small buffer for safety
+    timing.Set("wait_before_ms", std::max(100, delay_ms / 2));
+    timing.Set("observed_delay_ms", delay_ms);
+  } else {
+    timing.Set("wait_before_ms", 500);  // Default wait
+  }
+
+  // Wait for element to be visible/interactable
+  timing.Set("wait_for_element", true);
+  timing.Set("element_timeout_ms", 5000);
+
+  return timing;
+}
+
+// Singleton
+WorkflowGenerator& GetWorkflowGenerator() {
+  static base::NoDestructor<WorkflowGenerator> instance;
+  return *instance;
+}
+
+// Convenience functions
+std::string PatternToWorkflowJson(const ActionSequence& pattern) {
+  auto workflow = GetWorkflowGenerator().GenerateWorkflow(pattern);
+  return GetWorkflowGenerator().ExportToJson(workflow);
+}
+
+std::string GenerateWorkflowId() {
+  return "workflow_" + base::Uuid::GenerateRandomV4().AsLowercaseString();
+}
+
+bool IsValidWorkflowStep(const base::Value::Dict& step) {
+  // Must have id and action
+  if (!step.contains("id") || !step.contains("action")) {
+    return false;
+  }
+
+  // Action must be a known type
+  const std::string* action = step.FindString("action");
+  if (!action) {
+    return false;
+  }
+
+  // Check against known action types
+  static const std::set<std::string> known_actions = {
+      workflow_actions::kClick,    workflow_actions::kType,
+      workflow_actions::kNavigate, workflow_actions::kWait,
+      workflow_actions::kScroll,   workflow_actions::kSelect,
+      workflow_actions::kSubmit,   workflow_actions::kKeyPress,
+      workflow_actions::kHover,    workflow_actions::kDragDrop,
+      workflow_actions::kScreenshot, workflow_actions::kExtract,
+      workflow_actions::kAssert,
+  };
+
+  return known_actions.count(*action) > 0;
+}
+
+}  // namespace browseros::ghost_mode
