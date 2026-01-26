diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_types.cc b/chrome/browser/browseros/ghost_mode/ghost_mode_types.cc
new file mode 100644
index 0000000000000..c3d4e5f6a7b8c
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_types.cc
@@ -0,0 +1,246 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+
+#include "base/json/json_reader.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/strings/string_util.h"
+
+namespace browseros::ghost_mode {
+
+std::string ActionTypeToString(ActionType type) {
+  switch (type) {
+    case ActionType::kClick:
+      return "click";
+    case ActionType::kType:
+      return "type";
+    case ActionType::kNavigate:
+      return "navigate";
+    case ActionType::kScroll:
+      return "scroll";
+    case ActionType::kSelect:
+      return "select";
+    case ActionType::kSubmit:
+      return "submit";
+    case ActionType::kKeyPress:
+      return "keypress";
+    case ActionType::kHover:
+      return "hover";
+    case ActionType::kDragDrop:
+      return "dragdrop";
+  }
+  return "unknown";
+}
+
+ActionType StringToActionType(const std::string& str) {
+  std::string lower = base::ToLowerASCII(str);
+  
+  if (lower == "click") {
+    return ActionType::kClick;
+  } else if (lower == "type") {
+    return ActionType::kType;
+  } else if (lower == "navigate") {
+    return ActionType::kNavigate;
+  } else if (lower == "scroll") {
+    return ActionType::kScroll;
+  } else if (lower == "select") {
+    return ActionType::kSelect;
+  } else if (lower == "submit") {
+    return ActionType::kSubmit;
+  } else if (lower == "keypress") {
+    return ActionType::kKeyPress;
+  } else if (lower == "hover") {
+    return ActionType::kHover;
+  } else if (lower == "dragdrop") {
+    return ActionType::kDragDrop;
+  }
+  
+  LOG(WARNING) << "browseros: Unknown action type: " << str;
+  return ActionType::kClick;  // Default
+}
+
+base::Value::Dict RecordedAction::ToValue() const {
+  base::Value::Dict dict;
+  
+  dict.Set("id", id);
+  dict.Set("type", ActionTypeToString(type));
+  dict.Set("url", url.spec());
+  dict.Set("url_pattern", url_pattern);
+  
+  // Selectors as list
+  base::Value::List selectors_list;
+  for (const auto& sel : selectors) {
+    selectors_list.Append(sel);
+  }
+  dict.Set("selectors", std::move(selectors_list));
+  
+  dict.Set("element_text", element_text);
+  dict.Set("value", value);
+  dict.Set("is_parameterizable", is_parameterizable);
+  dict.Set("timestamp", timestamp.InMillisecondsSinceUnixEpoch());
+  dict.Set("tab_id", tab_id);
+  dict.Set("session_id", session_id);
+  dict.Set("time_since_previous", time_since_previous.InMilliseconds());
+  dict.Set("metadata", metadata.Clone());
+  
+  return dict;
+}
+
+std::optional<RecordedAction> RecordedAction::FromValue(
+    const base::Value::Dict& dict) {
+  RecordedAction action;
+  
+  const std::string* id = dict.FindString("id");
+  if (!id) {
+    return std::nullopt;
+  }
+  action.id = *id;
+  
+  const std::string* type_str = dict.FindString("type");
+  if (type_str) {
+    action.type = StringToActionType(*type_str);
+  }
+  
+  const std::string* url_str = dict.FindString("url");
+  if (url_str) {
+    action.url = GURL(*url_str);
+  }
+  
+  const std::string* url_pattern = dict.FindString("url_pattern");
+  if (url_pattern) {
+    action.url_pattern = *url_pattern;
+  }
+  
+  const base::Value::List* selectors_list = dict.FindList("selectors");
+  if (selectors_list) {
+    for (const auto& item : *selectors_list) {
+      if (item.is_string()) {
+        action.selectors.push_back(item.GetString());
+      }
+    }
+  }
+  
+  const std::string* element_text = dict.FindString("element_text");
+  if (element_text) {
+    action.element_text = *element_text;
+  }
+  
+  const std::string* value = dict.FindString("value");
+  if (value) {
+    action.value = *value;
+  }
+  
+  std::optional<bool> is_param = dict.FindBool("is_parameterizable");
+  if (is_param) {
+    action.is_parameterizable = *is_param;
+  }
+  
+  std::optional<double> timestamp_ms = dict.FindDouble("timestamp");
+  if (timestamp_ms) {
+    action.timestamp = base::Time::FromMillisecondsSinceUnixEpoch(
+        static_cast<int64_t>(*timestamp_ms));
+  }
+  
+  std::optional<int> tab_id = dict.FindInt("tab_id");
+  if (tab_id) {
+    action.tab_id = *tab_id;
+  }
+  
+  const std::string* session_id = dict.FindString("session_id");
+  if (session_id) {
+    action.session_id = *session_id;
+  }
+  
+  std::optional<double> time_since = dict.FindDouble("time_since_previous");
+  if (time_since) {
+    action.time_since_previous =
+        base::Milliseconds(static_cast<int64_t>(*time_since));
+  }
+  
+  const base::Value::Dict* metadata = dict.FindDict("metadata");
+  if (metadata) {
+    action.metadata = metadata->Clone();
+  }
+  
+  return action;
+}
+
+base::Value::Dict ActionSequence::ToValue() const {
+  base::Value::Dict dict;
+  
+  dict.Set("id", id);
+  dict.Set("name", name);
+  dict.Set("description", description);
+  
+  // Actions as list
+  base::Value::List actions_list;
+  for (const auto& action : actions) {
+    actions_list.Append(action.ToValue());
+  }
+  dict.Set("actions", std::move(actions_list));
+  
+  dict.Set("occurrence_count", occurrence_count);
+  dict.Set("first_seen", first_seen.InMillisecondsSinceUnixEpoch());
+  dict.Set("last_seen", last_seen.InMillisecondsSinceUnixEpoch());
+  dict.Set("confidence_score", confidence_score);
+  dict.Set("status", static_cast<int>(status));
+  dict.Set("url_pattern", url_pattern);
+  dict.Set("pattern_hash", pattern_hash);
+  dict.Set("is_dismissed", is_dismissed);
+  dict.Set("is_converted", is_converted);
+  dict.Set("workflow_id", workflow_id);
+  dict.Set("metadata", metadata.Clone());
+  
+  return dict;
+}
+
+std::optional<ActionSequence> ActionSequence::FromValue(
+    const base::Value::Dict& dict) {
+  ActionSequence sequence;
+  
+  const std::string* id = dict.FindString("id");
+  if (id) {
+    sequence.id = *id;
+  }
+  
+  const std::string* name = dict.FindString("name");
+  if (name) {
+    sequence.name = *name;
+  }
+  
+  const std::string* description = dict.FindString("description");
+  if (description) {
+    sequence.description = *description;
+  }
+  
+  const base::Value::List* actions_list = dict.FindList("actions");
+  if (actions_list) {
+    for (const auto& item : *actions_list) {
+      if (item.is_dict()) {
+        auto action = RecordedAction::FromValue(item.GetDict());
+        if (action.has_value()) {
+          sequence.actions.push_back(std::move(*action));
+        }
+      }
+    }
+  }
+  
+  sequence.occurrence_count = dict.FindInt("occurrence_count").value_or(0);
+  
+  std::optional<double> first_seen = dict.FindDouble("first_seen");
+  if (first_seen) {
+    sequence.first_seen = base::Time::FromMillisecondsSinceUnixEpoch(
+        static_cast<int64_t>(*first_seen));
+  }
+  
+  std::optional<double> last_seen = dict.FindDouble("last_seen");
+  if (last_seen) {
+    sequence.last_seen = base::Time::FromMillisecondsSinceUnixEpoch(
+        static_cast<int64_t>(*last_seen));
+  }
+  
+  sequence.confidence_score = dict.FindDouble("confidence_score").value_or(0.0);
+  sequence.status = static_cast<PatternStatus>(
+      dict.FindInt("status").value_or(0));
+  
+  const std::string* url_pattern = dict.FindString("url_pattern");
+  if (url_pattern) {
+    sequence.url_pattern = *url_pattern;
+  }
+  
+  const std::string* pattern_hash = dict.FindString("pattern_hash");
+  if (pattern_hash) {
+    sequence.pattern_hash = *pattern_hash;
+  }
+  
+  sequence.is_dismissed = dict.FindBool("is_dismissed").value_or(false);
+  sequence.is_converted = dict.FindBool("is_converted").value_or(false);
+  
+  const std::string* workflow_id = dict.FindString("workflow_id");
+  if (workflow_id) {
+    sequence.workflow_id = *workflow_id;
+  }
+  
+  const base::Value::Dict* metadata = dict.FindDict("metadata");
+  if (metadata) {
+    sequence.metadata = metadata->Clone();
+  }
+  
+  return sequence;
+}
+
+std::vector<std::string> ActionSequence::GetActionSummary() const {
+  std::vector<std::string> summary;
+  
+  for (const auto& action : actions) {
+    std::string step;
+    
+    switch (action.type) {
+      case ActionType::kClick:
+        step = "Click on " + (action.element_text.empty()
+                                  ? "element"
+                                  : "\"" + action.element_text + "\"");
+        break;
+      case ActionType::kType:
+        step = "Type into " + (action.element_text.empty()
+                                   ? "field"
+                                   : "\"" + action.element_text + "\"");
+        break;
+      case ActionType::kNavigate:
+        step = "Navigate to " + action.url_pattern;
+        break;
+      case ActionType::kScroll:
+        step = "Scroll page";
+        break;
+      case ActionType::kSelect:
+        step = "Select from dropdown";
+        break;
+      case ActionType::kSubmit:
+        step = "Submit form";
+        break;
+      case ActionType::kKeyPress:
+        step = "Press key";
+        break;
+      case ActionType::kHover:
+        step = "Hover over " + (action.element_text.empty()
+                                    ? "element"
+                                    : "\"" + action.element_text + "\"");
+        break;
+      case ActionType::kDragDrop:
+        step = "Drag and drop";
+        break;
+    }
+    
+    summary.push_back(step);
+  }
+  
+  return summary;
+}
+
+}  // namespace browseros::ghost_mode
