diff --git a/chrome/browser/ui/webui/settings/ghost_mode_handler.h b/chrome/browser/ui/webui/settings/ghost_mode_handler.h
new file mode 100644
index 0000000000000..c9d0e1f2a3b4c
--- /dev/null
+++ b/chrome/browser/ui/webui/settings/ghost_mode_handler.h
@@ -0,0 +1,78 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_WEBUI_SETTINGS_GHOST_MODE_HANDLER_H_
+#define CHROME_BROWSER_UI_WEBUI_SETTINGS_GHOST_MODE_HANDLER_H_
+
+#include "base/memory/raw_ptr.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_service.h"
+#include "chrome/browser/ui/webui/settings/settings_page_ui_handler.h"
+
+class Profile;
+
+namespace settings {
+
+// WebUI handler for Ghost Mode settings page.
+// Handles communication between the settings UI and the GhostModeService.
+class GhostModeHandler : public SettingsPageUIHandler,
+                         public browseros::ghost_mode::GhostModeServiceObserver {
+ public:
+  explicit GhostModeHandler(Profile* profile);
+  GhostModeHandler(const GhostModeHandler&) = delete;
+  GhostModeHandler& operator=(const GhostModeHandler&) = delete;
+  ~GhostModeHandler() override;
+
+  // SettingsPageUIHandler:
+  void RegisterMessages() override;
+  void OnJavascriptAllowed() override;
+  void OnJavascriptDisallowed() override;
+
+  // GhostModeServiceObserver:
+  void OnGhostModeStateChanged(bool enabled) override;
+  void OnPatternDetected(
+      const browseros::ghost_mode::ActionSequence& pattern) override;
+  void OnWorkflowGenerated(const std::string& workflow_json) override;
+  void OnStatsUpdated(int actions, int patterns, int workflows) override;
+
+ private:
+  // Handler for getGhostModeStats
+  void HandleGetGhostModeStats(const base::Value::List& args);
+  
+  // Handler for getGhostModePatterns
+  void HandleGetGhostModePatterns(const base::Value::List& args);
+  
+  // Handler for convertPatternToWorkflow
+  void HandleConvertPatternToWorkflow(const base::Value::List& args);
+  
+  // Handler for deleteGhostModePattern
+  void HandleDeleteGhostModePattern(const base::Value::List& args);
+  
+  // Handler for clearGhostModeData
+  void HandleClearGhostModeData(const base::Value::List& args);
+  
+  // Convert ActionSequence to base::Value for sending to JS
+  base::Value::Dict PatternToValue(
+      const browseros::ghost_mode::ActionSequence& pattern);
+  
+  // Send stats update to the frontend
+  void SendStatsUpdate();
+  
+  // Send patterns list to the frontend
+  void SendPatternsUpdate();
+
+  raw_ptr<Profile> profile_;
+  raw_ptr<browseros::ghost_mode::GhostModeService> ghost_mode_service_ = nullptr;
+};
+
+}  // namespace settings
+
+#endif  // CHROME_BROWSER_UI_WEBUI_SETTINGS_GHOST_MODE_HANDLER_H_
