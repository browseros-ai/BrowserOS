diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_service.h b/chrome/browser/browseros/ghost_mode/ghost_mode_service.h
new file mode 100644
index 0000000000000..e5f6a7b8c9d0e
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_service.h
@@ -0,0 +1,168 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SERVICE_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SERVICE_H_
+
+#include <memory>
+#include <string>
+#include <vector>
+
+#include "base/memory/raw_ptr.h"
+#include "base/observer_list.h"
+#include "base/timer/timer.h"
+#include "chrome/browser/browseros/ghost_mode/action_recorder.h"
+#include "chrome/browser/browseros/ghost_mode/action_store.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_executor.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+#include "chrome/browser/browseros/ghost_mode/pattern_detector.h"
+#include "chrome/browser/browseros/ghost_mode/suggestion_controller.h"
+#include "chrome/browser/browseros/ghost_mode/workflow_generator.h"
+#include "components/keyed_service/core/keyed_service.h"
+
+class Profile;
+class PrefService;
+
+namespace content {
+class WebContents;
+}
+
+namespace browseros::ghost_mode {
+
+// Observer interface for Ghost Mode events
+class GhostModeServiceObserver {
+ public:
+  virtual ~GhostModeServiceObserver() = default;
+  
+  // Called when Ghost Mode is enabled/disabled
+  virtual void OnGhostModeStateChanged(bool enabled) {}
+  
+  // Called when a new pattern is detected
+  virtual void OnPatternDetected(const ActionSequence& pattern) {}
+  
+  // Called when a workflow is generated from a pattern
+  virtual void OnWorkflowGenerated(const std::string& workflow_json) {}
+  
+  // Called when statistics are updated
+  virtual void OnStatsUpdated(int actions, int patterns, int workflows) {}
+};
+
+// Main service for AI Ghost Mode functionality.
+// Coordinates action recording, pattern detection, suggestions, and execution.
+class GhostModeService : public KeyedService,
+                         public PatternDetectorObserver,
+                         public SuggestionControllerObserver {
+ public:
+  explicit GhostModeService(Profile* profile);
+  GhostModeService(const GhostModeService&) = delete;
+  GhostModeService& operator=(const GhostModeService&) = delete;
+  ~GhostModeService() override;
+
+  // KeyedService:
+  void Shutdown() override;
+
+  // Enable or disable Ghost Mode
+  void SetEnabled(bool enabled);
+  bool IsEnabled() const;
+
+  // Start observing a WebContents
+  void StartObserving(content::WebContents* web_contents);
+  
+  // Stop observing a WebContents
+  void StopObserving(content::WebContents* web_contents);
+
+  // Manual pattern detection trigger
+  void DetectPatterns();
+
+  // Get detected patterns
+  std::vector<ActionSequence> GetDetectedPatterns() const;
+
+  // Convert pattern to workflow JSON
+  std::string ConvertPatternToWorkflow(const std::string& pattern_id);
+
+  // Execute a workflow in ghost mode
+  void ExecuteWorkflow(const std::string& workflow_json,
+                       GhostExecutor::CompletionCallback callback);
+
+  // Pause execution
+  void PauseExecution();
+  
+  // Resume execution
+  void ResumeExecution();
+
+  // Get statistics
+  struct Stats {
+    int total_actions = 0;
+    int total_patterns = 0;
+    int total_workflows = 0;
+  };
+  Stats GetStats() const;
+
+  // Clear all data
+  void ClearAllData();
+
+  // Add/remove domain exclusion
+  void AddExcludedDomain(const std::string& domain);
+  void RemoveExcludedDomain(const std::string& domain);
+  std::vector<std::string> GetExcludedDomains() const;
+
+  // Delete a specific pattern
+  void DeletePattern(const std::string& pattern_id);
+
+  // Dismiss pattern permanently (won't suggest again)
+  void DismissPattern(const std::string& pattern_id);
+
+  // Observer management
+  void AddObserver(GhostModeServiceObserver* observer);
+  void RemoveObserver(GhostModeServiceObserver* observer);
+
+  // PatternDetectorObserver:
+  void OnPatternDetected(const ActionSequence& pattern) override;
+  void OnDetectionComplete(int patterns_found) override;
+
+  // SuggestionControllerObserver:
+  void OnSuggestionAccepted(const ActionSequence& pattern) override;
+  void OnSuggestionDismissed(const std::string& pattern_id) override;
+  void OnSuggestionDeferred(const std::string& pattern_id) override;
+
+ private:
+  // Initialize components
+  void Initialize();
+  
+  // Periodic pattern detection
+  void SchedulePatternDetection();
+  void OnPatternDetectionTimer();
+  
+  // Periodic data cleanup
+  void ScheduleDataCleanup();
+  void OnDataCleanupTimer();
+  
+  // Notify observers
+  void NotifyStateChanged(bool enabled);
+  void NotifyPatternDetected(const ActionSequence& pattern);
+  void NotifyWorkflowGenerated(const std::string& json);
+  void NotifyStatsUpdated();
+
+  // Profile that owns this service
+  raw_ptr<Profile> profile_;
+  raw_ptr<PrefService> pref_service_;
+  
+  // Core components
+  std::unique_ptr<ActionStore> action_store_;
+  std::unique_ptr<PatternDetector> pattern_detector_;
+  std::unique_ptr<SuggestionController> suggestion_controller_;
+  std::unique_ptr<WorkflowGenerator> workflow_generator_;
+  std::unique_ptr<GhostExecutor> ghost_executor_;
+  
+  // Active recorders for observed WebContents
+  std::map<content::WebContents*, std::unique_ptr<ActionRecorder>> recorders_;
+  
+  // Timers for periodic tasks
+  base::RepeatingTimer pattern_detection_timer_;
+  base::RepeatingTimer data_cleanup_timer_;
+  
+  // Observers
+  base::ObserverList<GhostModeServiceObserver> observers_;
+};
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SERVICE_H_
