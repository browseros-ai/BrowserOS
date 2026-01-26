diff --git a/chrome/browser/ui/webui/settings/ghost_mode_handler.cc b/chrome/browser/ui/webui/settings/ghost_mode_handler.cc
new file mode 100644
index 0000000000000..d0e1f2a3b4c5d
--- /dev/null
+++ b/chrome/browser/ui/webui/settings/ghost_mode_handler.cc
@@ -0,0 +1,162 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/webui/settings/ghost_mode_handler.h"
+
+#include <utility>
+
+#include "base/functional/bind.h"
+#include "base/values.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_service_factory.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+#include "chrome/browser/profiles/profile.h"
+
+namespace settings {
+
+GhostModeHandler::GhostModeHandler(Profile* profile)
+    : profile_(profile) {}
+
+GhostModeHandler::~GhostModeHandler() {
+  if (ghost_mode_service_) {
+    ghost_mode_service_->RemoveObserver(this);
+  }
+}
+
+void GhostModeHandler::RegisterMessages() {
+  web_ui()->RegisterMessageCallback(
+      "getGhostModeStats",
+      base::BindRepeating(&GhostModeHandler::HandleGetGhostModeStats,
+                          base::Unretained(this)));
+  
+  web_ui()->RegisterMessageCallback(
+      "getGhostModePatterns",
+      base::BindRepeating(&GhostModeHandler::HandleGetGhostModePatterns,
+                          base::Unretained(this)));
+  
+  web_ui()->RegisterMessageCallback(
+      "convertPatternToWorkflow",
+      base::BindRepeating(&GhostModeHandler::HandleConvertPatternToWorkflow,
+                          base::Unretained(this)));
+  
+  web_ui()->RegisterMessageCallback(
+      "deleteGhostModePattern",
+      base::BindRepeating(&GhostModeHandler::HandleDeleteGhostModePattern,
+                          base::Unretained(this)));
+  
+  web_ui()->RegisterMessageCallback(
+      "clearGhostModeData",
+      base::BindRepeating(&GhostModeHandler::HandleClearGhostModeData,
+                          base::Unretained(this)));
+}
+
+void GhostModeHandler::OnJavascriptAllowed() {
+  ghost_mode_service_ = 
+      browseros::ghost_mode::GhostModeServiceFactory::GetForProfile(profile_);
+  
+  if (ghost_mode_service_) {
+    ghost_mode_service_->AddObserver(this);
+  }
+}
+
+void GhostModeHandler::OnJavascriptDisallowed() {
+  if (ghost_mode_service_) {
+    ghost_mode_service_->RemoveObserver(this);
+  }
+}
+
+void GhostModeHandler::OnGhostModeStateChanged(bool enabled) {
+  FireWebUIListener("ghost-mode-state-changed", base::Value(enabled));
+}
+
+void GhostModeHandler::OnPatternDetected(
+    const browseros::ghost_mode::ActionSequence& pattern) {
+  FireWebUIListener("ghost-mode-pattern-detected", PatternToValue(pattern));
+}
+
+void GhostModeHandler::OnWorkflowGenerated(const std::string& workflow_json) {
+  FireWebUIListener("ghost-mode-workflow-generated",
+                    base::Value(workflow_json));
+}
+
+void GhostModeHandler::OnStatsUpdated(int actions, int patterns, int workflows) {
+  base::Value::Dict stats;
+  stats.Set("actions", actions);
+  stats.Set("patterns", patterns);
+  stats.Set("workflows", workflows);
+  FireWebUIListener("ghost-mode-stats-updated", std::move(stats));
+}
+
+void GhostModeHandler::HandleGetGhostModeStats(const base::Value::List& args) {
+  AllowJavascript();
+  SendStatsUpdate();
+}
+
+void GhostModeHandler::HandleGetGhostModePatterns(const base::Value::List& args) {
+  AllowJavascript();
+  SendPatternsUpdate();
+}
+
+void GhostModeHandler::HandleConvertPatternToWorkflow(
+    const base::Value::List& args) {
+  if (args.empty() || !args[0].is_string()) {
+    return;
+  }
+  
+  const std::string& pattern_id = args[0].GetString();
+  
+  if (ghost_mode_service_) {
+    std::string workflow_json = 
+        ghost_mode_service_->ConvertPatternToWorkflow(pattern_id);
+    
+    if (!workflow_json.empty()) {
+      FireWebUIListener("ghost-mode-workflow-created",
+                        base::Value(workflow_json));
+    }
+  }
+}
+
+void GhostModeHandler::HandleDeleteGhostModePattern(
+    const base::Value::List& args) {
+  if (args.empty() || !args[0].is_string()) {
+    return;
+  }
+  
+  const std::string& pattern_id = args[0].GetString();
+  
+  if (ghost_mode_service_) {
+    ghost_mode_service_->DeletePattern(pattern_id);
+    SendPatternsUpdate();
+  }
+}
+
+void GhostModeHandler::HandleClearGhostModeData(const base::Value::List& args) {
+  if (ghost_mode_service_) {
+    ghost_mode_service_->ClearAllData();
+    SendStatsUpdate();
+    SendPatternsUpdate();
+  }
+}
+
+base::Value::Dict GhostModeHandler::PatternToValue(
+    const browseros::ghost_mode::ActionSequence& pattern) {
+  base::Value::Dict dict;
+  dict.Set("id", pattern.id);
+  dict.Set("name", pattern.name);
+  dict.Set("occurrence_count", pattern.occurrence_count);
+  dict.Set("confidence_score", pattern.confidence_score);
+  dict.Set("first_seen", pattern.first_seen.InMillisecondsFSinceUnixEpoch());
+  dict.Set("last_seen", pattern.last_seen.InMillisecondsFSinceUnixEpoch());
+  
+  base::Value::List actions_list;
+  for (const auto& action : pattern.actions) {
+    base::Value::Dict action_dict;
+    action_dict.Set("type", static_cast<int>(action.type));
+    action_dict.Set("url", action.url.spec());
+    if (!action.selectors.empty()) {
+      action_dict.Set("selector", action.selectors[0]);
+    }
+    actions_list.Append(std::move(action_dict));
+  }
+  dict.Set("actions", std::move(actions_list));
+  
+  return dict;
+}
+
+void GhostModeHandler::SendStatsUpdate() {
+  if (!ghost_mode_service_) {
+    return;
+  }
+  
+  auto stats = ghost_mode_service_->GetStats();
+  
+  base::Value::Dict dict;
+  dict.Set("actions", stats.total_actions);
+  dict.Set("patterns", stats.total_patterns);
+  dict.Set("workflows", stats.total_workflows);
+  
+  FireWebUIListener("ghost-mode-stats-received", std::move(dict));
+}
+
+void GhostModeHandler::SendPatternsUpdate() {
+  if (!ghost_mode_service_) {
+    return;
+  }
+  
+  auto patterns = ghost_mode_service_->GetDetectedPatterns();
+  
+  base::Value::List patterns_list;
+  for (const auto& pattern : patterns) {
+    patterns_list.Append(PatternToValue(pattern));
+  }
+  
+  FireWebUIListener("ghost-mode-patterns-received", std::move(patterns_list));
+}
+
+}  // namespace settings
