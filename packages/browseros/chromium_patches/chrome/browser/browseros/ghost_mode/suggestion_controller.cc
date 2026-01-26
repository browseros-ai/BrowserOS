diff --git a/chrome/browser/browseros/ghost_mode/suggestion_controller.cc b/chrome/browser/browseros/ghost_mode/suggestion_controller.cc
new file mode 100644
index 0000000000000..a7b8c9d0e1f2a
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/suggestion_controller.cc
@@ -0,0 +1,237 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/suggestion_controller.h"
+
+#include "base/logging.h"
+#include "chrome/browser/browseros/ghost_mode/action_store.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h"
+#include "chrome/browser/browseros/ghost_mode/pattern_detector.h"
+#include "chrome/browser/browseros/ghost_mode/pattern_matcher.h"
+#include "components/prefs/pref_service.h"
+#include "content/public/browser/web_contents.h"
+
+namespace browseros::ghost_mode {
+
+SuggestionController::SuggestionController(ActionStore* action_store,
+                                            PatternDetector* pattern_detector,
+                                            PrefService* pref_service)
+    : action_store_(action_store),
+      pattern_detector_(pattern_detector),
+      pref_service_(pref_service) {
+  CHECK(action_store_);
+  CHECK(pattern_detector_);
+  CHECK(pref_service_);
+}
+
+SuggestionController::~SuggestionController() {
+  Stop();
+}
+
+void SuggestionController::Start() {
+  if (is_running_) {
+    return;
+  }
+
+  // Check if Ghost Mode is enabled
+  if (!pref_service_->GetBoolean(prefs::kGhostModeEnabled)) {
+    VLOG(1) << "browseros: Ghost Mode is disabled, not starting suggestions";
+    return;
+  }
+
+  VLOG(1) << "browseros: Starting Ghost Mode suggestion controller";
+
+  // Register as observer for pattern detection
+  pattern_detector_->AddObserver(this);
+
+  // Load existing pending suggestions
+  auto patterns = action_store_->GetAllPatterns();
+  for (const auto& pattern : patterns) {
+    if (pattern.status == PatternStatus::kNew ||
+        pattern.status == PatternStatus::kPending) {
+      pending_suggestions_.push_back(pattern);
+    }
+  }
+
+  // Start periodic detection
+  ScheduleNextDetection();
+
+  is_running_ = true;
+  LOG(INFO) << "browseros: Ghost Mode suggestion controller started with "
+            << pending_suggestions_.size() << " pending suggestions";
+}
+
+void SuggestionController::Stop() {
+  if (!is_running_) {
+    return;
+  }
+
+  VLOG(1) << "browseros: Stopping Ghost Mode suggestion controller";
+
+  detection_timer_.Stop();
+  pattern_detector_->RemoveObserver(this);
+  
+  is_running_ = false;
+}
+
+std::vector<ActionSequence> SuggestionController::GetPendingSuggestions() {
+  return pending_suggestions_;
+}
+
+std::optional<ActionSequence> SuggestionController::GetSuggestionForUrl(
+    const GURL& url) {
+  if (!url.is_valid()) {
+    return std::nullopt;
+  }
+
+  // Find patterns that match this URL
+  for (const auto& pattern : pending_suggestions_) {
+    if (GetPatternMatcher().UrlMatchesPattern(url, pattern.url_pattern)) {
+      if (pattern.confidence_score >= min_confidence_for_suggestion_) {
+        return pattern;
+      }
+    }
+  }
+
+  return std::nullopt;
+}
+
+void SuggestionController::HandleResponse(const std::string& pattern_id,
+                                           SuggestionResponse response) {
+  auto pattern = action_store_->GetPattern(pattern_id);
+  if (!pattern.has_value()) {
+    LOG(WARNING) << "browseros: Pattern not found: " << pattern_id;
+    return;
+  }
+
+  VLOG(1) << "browseros: Handling response for pattern: " << pattern->name;
+
+  switch (response) {
+    case SuggestionResponse::kAccept: {
+      // Mark as converted and notify observers
+      pattern->status = PatternStatus::kConverted;
+      action_store_->UpdatePattern(*pattern);
+      
+      // Remove from pending
+      pending_suggestions_.erase(
+          std::remove_if(pending_suggestions_.begin(),
+                         pending_suggestions_.end(),
+                         [&pattern_id](const ActionSequence& p) {
+                           return p.id == pattern_id;
+                         }),
+          pending_suggestions_.end());
+      
+      for (auto& observer : observers_) {
+        observer.OnSuggestionAccepted(pattern_id);
+      }
+      
+      LOG(INFO) << "browseros: Pattern accepted: " << pattern->name;
+      break;
+    }
+
+    case SuggestionResponse::kDismiss: {
+      // Mark as dismissed
+      action_store_->DismissPattern(pattern_id);
+      
+      // Remove from pending
+      pending_suggestions_.erase(
+          std::remove_if(pending_suggestions_.begin(),
+                         pending_suggestions_.end(),
+                         [&pattern_id](const ActionSequence& p) {
+                           return p.id == pattern_id;
+                         }),
+          pending_suggestions_.end());
+      
+      for (auto& observer : observers_) {
+        observer.OnSuggestionDismissed(pattern_id);
+      }
+      
+      VLOG(1) << "browseros: Pattern dismissed: " << pattern->name;
+      break;
+    }
+
+    case SuggestionResponse::kLater: {
+      // Just hide for now, keep in pending
+      HideSuggestion();
+      VLOG(1) << "browseros: Pattern deferred: " << pattern->name;
+      break;
+    }
+
+    case SuggestionResponse::kCustomize: {
+      // Open workflow editor with this pattern pre-filled
+      // This will be handled by the UI layer
+      VLOG(1) << "browseros: Opening customization for: " << pattern->name;
+      break;
+    }
+  }
+}
+
+void SuggestionController::ShowSuggestion(const ActionSequence& pattern,
+                                           content::WebContents* web_contents) {
+  if (!web_contents) {
+    return;
+  }
+
+  VLOG(1) << "browseros: Showing suggestion for: " << pattern.name;
+
+  // Update last suggestion time
+  last_suggestion_time_ = base::Time::Now();
+
+  // Notify observers to display UI
+  NotifySuggestionAvailable(pattern);
+  NotifyVisibilityChanged(true);
+}
+
+void SuggestionController::HideSuggestion() {
+  NotifyVisibilityChanged(false);
+}
+
+void SuggestionController::AddObserver(SuggestionObserver* observer) {
+  observers_.AddObserver(observer);
+}
+
+void SuggestionController::RemoveObserver(SuggestionObserver* observer) {
+  observers_.RemoveObserver(observer);
+}
+
+void SuggestionController::OnPatternDetected(const ActionSequence& pattern) {
+  VLOG(1) << "browseros: New pattern detected: " << pattern.name;
+
+  // Check if it's already in pending
+  for (const auto& existing : pending_suggestions_) {
+    if (existing.pattern_hash == pattern.pattern_hash) {
+      return;  // Already have this pattern
+    }
+  }
+
+  // Save to store
+  action_store_->SavePattern(pattern);
+
+  // Add to pending
+  pending_suggestions_.push_back(pattern);
+
+  // Optionally show suggestion immediately
+  if (ShouldShowSuggestion(pattern)) {
+    NotifySuggestionAvailable(pattern);
+  }
+}
+
+void SuggestionController::OnDetectionComplete(int patterns_found) {
+  VLOG(1) << "browseros: Detection complete, found " << patterns_found
+          << " patterns";
+}
+
+void SuggestionController::SetMinConfidenceForSuggestion(double confidence) {
+  min_confidence_for_suggestion_ = confidence;
+}
+
+void SuggestionController::SetCooldownPeriod(base::TimeDelta cooldown) {
+  suggestion_cooldown_ = cooldown;
+}
+
+void SuggestionController::RunDetection() {
+  if (!is_running_) {
+    return;
+  }
+
+  VLOG(1) << "browseros: Running scheduled pattern detection";
+  pattern_detector_->DetectPatterns();
+}
+
+void SuggestionController::ScheduleNextDetection() {
+  detection_timer_.Start(FROM_HERE, detection_interval_,
+                          base::BindRepeating(&SuggestionController::RunDetection,
+                                               weak_factory_.GetWeakPtr()));
+}
+
+bool SuggestionController::ShouldShowSuggestion(const ActionSequence& pattern) {
+  // Check confidence threshold
+  if (pattern.confidence_score < min_confidence_for_suggestion_) {
+    return false;
+  }
+
+  // Check cooldown
+  if (!last_suggestion_time_.is_null()) {
+    base::TimeDelta since_last = base::Time::Now() - last_suggestion_time_;
+    if (since_last < suggestion_cooldown_) {
+      return false;
+    }
+  }
+
+  return true;
+}
+
+void SuggestionController::NotifySuggestionAvailable(
+    const ActionSequence& pattern) {
+  for (auto& observer : observers_) {
+    observer.OnSuggestionAvailable(pattern);
+  }
+}
+
+void SuggestionController::NotifyVisibilityChanged(bool visible) {
+  for (auto& observer : observers_) {
+    observer.OnSuggestionVisibilityChanged(visible);
+  }
+}
+
+}  // namespace browseros::ghost_mode
