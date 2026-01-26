diff --git a/chrome/browser/browseros/ghost_mode/suggestion_controller.h b/chrome/browser/browseros/ghost_mode/suggestion_controller.h
new file mode 100644
index 0000000000000..f6a7b8c9d0e1f
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/suggestion_controller.h
@@ -0,0 +1,142 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_SUGGESTION_CONTROLLER_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_SUGGESTION_CONTROLLER_H_
+
+#include <memory>
+#include <string>
+#include <vector>
+
+#include "base/callback_forward.h"
+#include "base/memory/raw_ptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/observer_list.h"
+#include "base/timer/timer.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+#include "chrome/browser/browseros/ghost_mode/pattern_detector.h"
+
+class PrefService;
+
+namespace content {
+class WebContents;
+}  // namespace content
+
+namespace browseros::ghost_mode {
+
+class ActionStore;
+class PatternDetector;
+
+// Observer for suggestion events
+class SuggestionObserver {
+ public:
+  virtual ~SuggestionObserver() = default;
+  
+  // Called when a new pattern suggestion is available
+  virtual void OnSuggestionAvailable(const ActionSequence& pattern) {}
+  
+  // Called when user accepts a suggestion
+  virtual void OnSuggestionAccepted(const std::string& pattern_id) {}
+  
+  // Called when user dismisses a suggestion
+  virtual void OnSuggestionDismissed(const std::string& pattern_id) {}
+  
+  // Called when suggestion UI should be shown/hidden
+  virtual void OnSuggestionVisibilityChanged(bool visible) {}
+};
+
+// User response to a suggestion
+enum class SuggestionResponse {
+  kAccept,     // Convert to workflow
+  kDismiss,    // Don't show again
+  kLater,      // Hide for now, show again later
+  kCustomize,  // Open editor to customize
+};
+
+// SuggestionController manages the UI for showing Ghost Mode suggestions
+// to the user. It observes the PatternDetector and displays notifications
+// when significant patterns are detected.
+//
+// UI Options:
+// 1. InfoBar at top of page
+// 2. Omnibox chip/badge
+// 3. Side panel notification
+// 4. Browser notification (for background detection)
+class SuggestionController : public PatternDetectorObserver {
+ public:
+  SuggestionController(ActionStore* action_store,
+                       PatternDetector* pattern_detector,
+                       PrefService* pref_service);
+  ~SuggestionController() override;
+
+  // Start/stop the suggestion system
+  void Start();
+  void Stop();
+  bool IsRunning() const { return is_running_; }
+
+  // Get pending suggestions (patterns not yet shown/dismissed)
+  std::vector<ActionSequence> GetPendingSuggestions();
+
+  // Get the top suggestion for the current page (if any)
+  std::optional<ActionSequence> GetSuggestionForUrl(const GURL& url);
+
+  // Handle user response to a suggestion
+  void HandleResponse(const std::string& pattern_id,
+                      SuggestionResponse response);
+
+  // Show suggestion UI for a specific pattern
+  void ShowSuggestion(const ActionSequence& pattern,
+                      content::WebContents* web_contents);
+
+  // Hide any visible suggestion UI
+  void HideSuggestion();
+
+  // Observer management
+  void AddObserver(SuggestionObserver* observer);
+  void RemoveObserver(SuggestionObserver* observer);
+
+  // PatternDetectorObserver implementation
+  void OnPatternDetected(const ActionSequence& pattern) override;
+  void OnDetectionComplete(int patterns_found) override;
+
+  // Configuration
+  void SetMinConfidenceForSuggestion(double confidence);
+  void SetCooldownPeriod(base::TimeDelta cooldown);
+
+ private:
+  // Run periodic pattern detection
+  void RunDetection();
+
+  // Schedule next detection run
+  void ScheduleNextDetection();
+
+  // Check if we should show suggestion now
+  bool ShouldShowSuggestion(const ActionSequence& pattern);
+
+  // Notify observers
+  void NotifySuggestionAvailable(const ActionSequence& pattern);
+  void NotifyVisibilityChanged(bool visible);
+
+  // Dependencies
+  raw_ptr<ActionStore> action_store_;
+  raw_ptr<PatternDetector> pattern_detector_;
+  raw_ptr<PrefService> pref_service_;
+
+  // State
+  bool is_running_ = false;
+  std::vector<ActionSequence> pending_suggestions_;
+  base::Time last_suggestion_time_;
+
+  // Configuration
+  double min_confidence_for_suggestion_ = 0.85;
+  base::TimeDelta detection_interval_ = base::Hours(1);
+  base::TimeDelta suggestion_cooldown_ = base::Minutes(30);
+
+  // Timer for periodic detection
+  base::RepeatingTimer detection_timer_;
+
+  // Observers
+  base::ObserverList<SuggestionObserver> observers_;
+
+  // Weak pointer factory
+  base::WeakPtrFactory<SuggestionController> weak_factory_{this};
+};
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_SUGGESTION_CONTROLLER_H_
