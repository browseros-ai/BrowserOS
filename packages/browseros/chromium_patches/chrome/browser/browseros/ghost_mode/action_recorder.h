diff --git a/chrome/browser/browseros/ghost_mode/action_recorder.h b/chrome/browser/browseros/ghost_mode/action_recorder.h
new file mode 100644
index 0000000000000..7f8a9b0c1d2e3
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/action_recorder.h
@@ -0,0 +1,142 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_ACTION_RECORDER_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_ACTION_RECORDER_H_
+
+#include <memory>
+#include <string>
+
+#include "base/memory/raw_ptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/observer_list.h"
+#include "base/time/time.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+#include "content/public/browser/web_contents_observer.h"
+#include "url/gurl.h"
+
+class PrefService;
+
+namespace content {
+class WebContents;
+}
+
+namespace browseros::ghost_mode {
+
+class ActionStore;
+
+// Observer interface for action recording events
+class ActionRecorderObserver {
+ public:
+  virtual ~ActionRecorderObserver() = default;
+  
+  // Called when an action is recorded
+  virtual void OnActionRecorded(const RecordedAction& action) {}
+  
+  // Called when a new session starts
+  virtual void OnSessionStarted(const std::string& session_id) {}
+  
+  // Called when recording is enabled/disabled
+  virtual void OnRecordingStateChanged(bool enabled) {}
+};
+
+// ActionRecorder observes user interactions with web pages and records
+// them to ActionStore for pattern detection.
+//
+// It integrates with the existing BrowserOS content observation infrastructure
+// and adds Ghost Mode specific recording logic.
+//
+// Privacy: This class uses SensitiveDetector to ensure no sensitive data
+// (passwords, credit cards, etc.) is ever recorded.
+class ActionRecorder : public content::WebContentsObserver {
+ public:
+  ActionRecorder(content::WebContents* web_contents,
+                 PrefService* pref_service,
+                 ActionStore* action_store);
+  ~ActionRecorder() override;
+
+  // Start/stop recording for this WebContents
+  void StartRecording();
+  void StopRecording();
+  bool IsRecording() const { return is_recording_; }
+
+  // Get current session ID
+  const std::string& session_id() const { return session_id_; }
+
+  // Observer management
+  void AddObserver(ActionRecorderObserver* observer);
+  void RemoveObserver(ActionRecorderObserver* observer);
+
+  // Record specific action types (called from event handlers)
+  void RecordClick(const std::string& selector,
+                   const std::string& element_text,
+                   int x, int y);
+  
+  void RecordType(const std::string& selector,
+                  const std::string& value,
+                  const std::string& input_type,
+                  const std::string& name,
+                  const std::string& id,
+                  const std::string& autocomplete);
+  
+  void RecordNavigation(const GURL& url, bool is_user_initiated);
+  
+  void RecordScroll(int delta_x, int delta_y, int scroll_x, int scroll_y);
+  
+  void RecordSubmit(const std::string& form_selector);
+  
+  void RecordKeyPress(const std::string& key,
+                      bool ctrl, bool alt, bool shift, bool meta);
+
+  // content::WebContentsObserver overrides
+  void DidFinishNavigation(
+      content::NavigationHandle* navigation_handle) override;
+  void WebContentsDestroyed() override;
+
+ private:
+  // Create a new session ID
+  void StartNewSession();
+  
+  // Check if recording is allowed for current page
+  bool ShouldRecordForCurrentPage() const;
+  
+  // Create base RecordedAction with common fields filled
+  RecordedAction CreateBaseAction(ActionType type);
+  
+  // Store action and notify observers
+  void StoreAction(RecordedAction action);
+  
+  // Normalize URL for pattern matching
+  std::string NormalizeUrl(const GURL& url) const;
+  
+  // Generate stable selectors for an element
+  std::vector<std::string> GenerateSelectors(const std::string& primary_selector);
+
+  // Whether recording is currently active
+  bool is_recording_ = false;
+  
+  // Current browsing session ID
+  std::string session_id_;
+  
+  // Timestamp of last recorded action (for time_since_previous)
+  base::Time last_action_time_;
+  
+  // Current page URL
+  GURL current_url_;
+  
+  // Scroll throttling - only record scroll events at most once per 500ms
+  base::Time last_scroll_time_;
+  static constexpr base::TimeDelta kScrollThrottleInterval = base::Milliseconds(500);
+  
+  // Dependencies (not owned)
+  raw_ptr<PrefService> pref_service_;
+  raw_ptr<ActionStore> action_store_;
+  
+  // Observers
+  base::ObserverList<ActionRecorderObserver> observers_;
+  
+  // Weak pointer factory
+  base::WeakPtrFactory<ActionRecorder> weak_factory_{this};
+};
+
+// Factory function to create ActionRecorder for a WebContents
+std::unique_ptr<ActionRecorder> CreateActionRecorder(
+    content::WebContents* web_contents,
+    PrefService* pref_service,
+    ActionStore* action_store);
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_ACTION_RECORDER_H_
