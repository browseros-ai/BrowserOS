diff --git a/chrome/browser/browseros/ghost_mode/action_recorder.cc b/chrome/browser/browseros/ghost_mode/action_recorder.cc
new file mode 100644
index 0000000000000..8a9b0c1d2e3f4
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/action_recorder.cc
@@ -0,0 +1,248 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/action_recorder.h"
+
+#include <cmath>
+
+#include "base/logging.h"
+#include "base/strings/string_util.h"
+#include "base/uuid.h"
+#include "chrome/browser/browseros/ghost_mode/action_store.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h"
+#include "chrome/browser/browseros/ghost_mode/sensitive_detector.h"
+#include "content/public/browser/navigation_handle.h"
+#include "content/public/browser/render_frame_host.h"
+#include "content/public/browser/web_contents.h"
+#include "url/gurl.h"
+
+namespace browseros::ghost_mode {
+
+ActionRecorder::ActionRecorder(content::WebContents* web_contents,
+                               PrefService* pref_service,
+                               ActionStore* action_store)
+    : content::WebContentsObserver(web_contents),
+      pref_service_(pref_service),
+      action_store_(action_store) {
+  CHECK(pref_service_);
+  CHECK(action_store_);
+  
+  // Initialize with current URL if available
+  if (web_contents) {
+    current_url_ = web_contents->GetLastCommittedURL();
+  }
+}
+
+ActionRecorder::~ActionRecorder() {
+  StopRecording();
+}
+
+void ActionRecorder::StartRecording() {
+  if (is_recording_) {
+    return;
+  }
+  
+  // Check if Ghost Mode is enabled
+  if (!IsGhostModeEnabled(pref_service_)) {
+    VLOG(1) << "[ghost_mode] Recording not started - Ghost Mode disabled";
+    return;
+  }
+  
+  // Check if recording is allowed for current page
+  if (!ShouldRecordForCurrentPage()) {
+    VLOG(1) << "[ghost_mode] Recording not started - page excluded";
+    return;
+  }
+  
+  is_recording_ = true;
+  StartNewSession();
+  
+  VLOG(1) << "[ghost_mode] Started recording, session: " << session_id_;
+  
+  for (auto& observer : observers_) {
+    observer.OnRecordingStateChanged(true);
+    observer.OnSessionStarted(session_id_);
+  }
+}
+
+void ActionRecorder::StopRecording() {
+  if (!is_recording_) {
+    return;
+  }
+  
+  is_recording_ = false;
+  VLOG(1) << "[ghost_mode] Stopped recording, session: " << session_id_;
+  
+  for (auto& observer : observers_) {
+    observer.OnRecordingStateChanged(false);
+  }
+}
+
+void ActionRecorder::StartNewSession() {
+  session_id_ = base::Uuid::GenerateRandomV4().AsLowercaseString();
+  last_action_time_ = base::Time::Now();
+}
+
+void ActionRecorder::AddObserver(ActionRecorderObserver* observer) {
+  observers_.AddObserver(observer);
+}
+
+void ActionRecorder::RemoveObserver(ActionRecorderObserver* observer) {
+  observers_.RemoveObserver(observer);
+}
+
+bool ActionRecorder::ShouldRecordForCurrentPage() const {
+  if (current_url_.is_empty()) {
+    return false;
+  }
+  
+  // Check if domain is excluded
+  if (IsDomainExcluded(pref_service_, current_url_.host())) {
+    return false;
+  }
+  
+  // Check if URL is sensitive (login, payment, etc.)
+  if (GetSensitiveDetector().IsSensitiveUrl(current_url_.spec())) {
+    return false;
+  }
+  
+  // Don't record internal pages
+  if (current_url_.SchemeIs("chrome") || 
+      current_url_.SchemeIs("chrome-extension") ||
+      current_url_.SchemeIs("devtools")) {
+    return false;
+  }
+  
+  return true;
+}
+
+RecordedAction ActionRecorder::CreateBaseAction(ActionType type) {
+  RecordedAction action;
+  action.id = base::Uuid::GenerateRandomV4().AsLowercaseString();
+  action.type = type;
+  action.url = current_url_;
+  action.url_pattern = NormalizeUrl(current_url_);
+  action.timestamp = base::Time::Now();
+  action.session_id = session_id_;
+  
+  if (web_contents()) {
+    // Store tab ID for grouping using stable frame tree node ID
+    action.tab_id = static_cast<int>(
+        web_contents()->GetPrimaryMainFrame()->GetFrameTreeNodeId().value());
+  }
+  
+  // Calculate time since previous action
+  if (!last_action_time_.is_null()) {
+    action.time_since_previous = action.timestamp - last_action_time_;
+  }
+  last_action_time_ = action.timestamp;
+  
+  return action;
+}
+
+std::string ActionRecorder::NormalizeUrl(const GURL& url) const {
+  // Strip query parameters and fragments for pattern matching
+  // Keep scheme, host, and path
+  GURL::Replacements replacements;
+  replacements.ClearQuery();
+  replacements.ClearRef();
+  return url.ReplaceComponents(replacements).spec();
+}
+
+std::vector<std::string> ActionRecorder::GenerateSelectors(
+    const std::string& primary_selector) {
+  std::vector<std::string> selectors;
+  
+  // Always include the primary selector first
+  if (!primary_selector.empty()) {
+    selectors.push_back(primary_selector);
+  }
+  
+  // Generate fallback selectors based on the primary type
+  // Priority: data-testid > id > aria-label > class-based > nth-child
+  
+  // If primary is a data-testid, try to derive other selectors
+  if (primary_selector.find("[data-testid") != std::string::npos) {
+    // Already the most stable, add class-based fallback if present
+    size_t class_pos = primary_selector.find(".");
+    if (class_pos != std::string::npos) {
+      selectors.push_back(primary_selector.substr(class_pos));
+    }
+  }
+  // If primary is an ID selector
+  else if (!primary_selector.empty() && primary_selector[0] == '#') {
+    // ID is pretty stable, could add tag + id combo
+    selectors.push_back(primary_selector);
+  }
+  // If primary is aria-label based
+  else if (primary_selector.find("[aria-label") != std::string::npos) {
+    // aria-label is accessibility-focused and fairly stable
+    // Try to add role-based selector
+    if (primary_selector.find("[role=") == std::string::npos) {
+      // Could add [role=button] or similar if we had the info
+    }
+  }
+  // If primary is class-based
+  else if (!primary_selector.empty() && primary_selector[0] == '.') {
+    // Classes can be unstable, add text-based fallback if possible
+    selectors.push_back(primary_selector);
+  }
+  
+  // Always ensure we have at least the primary
+  if (selectors.empty() && !primary_selector.empty()) {
+    selectors.push_back(primary_selector);
+  }
+  
+  return selectors;
+}
+
+void ActionRecorder::StoreAction(RecordedAction action) {
+  if (!is_recording_) {
+    return;
+  }
+  
+  VLOG(2) << "[ghost_mode] Recording action: " 
+          << ActionTypeToString(action.type)
+          << " on " << action.url_pattern;
+  
+  // Store to database
+  action_store_->AddAction(action);
+  
+  // Notify observers
+  for (auto& observer : observers_) {
+    observer.OnActionRecorded(action);
+  }
+}
+
+void ActionRecorder::RecordClick(const std::string& selector,
+                                  const std::string& element_text,
+                                  int x, int y) {
+  if (!is_recording_ || !ShouldRecordForCurrentPage()) {
+    return;
+  }
+  
+  RecordedAction action = CreateBaseAction(ActionType::kClick);
+  action.selectors = GenerateSelectors(selector);
+  action.element_text = element_text;
+  action.metadata.Set("x", x);
+  action.metadata.Set("y", y);
+  
+  StoreAction(std::move(action));
+}
+
+void ActionRecorder::RecordType(const std::string& selector,
+                                 const std::string& value,
+                                 const std::string& input_type,
+                                 const std::string& name,
+                                 const std::string& id,
+                                 const std::string& autocomplete) {
+  if (!is_recording_ || !ShouldRecordForCurrentPage()) {
+    return;
+  }
+  
+  // CRITICAL: Check if this is a sensitive field
+  if (ShouldSkipRecording(input_type, name, id, autocomplete,
+                          "", "", selector, current_url_.spec())) {
+    VLOG(1) << "[ghost_mode] Skipping sensitive field: " << name;
+    return;
+  }
+  
+  RecordedAction action = CreateBaseAction(ActionType::kType);
+  action.selectors = GenerateSelectors(selector);
+  action.value = value;
+  action.is_parameterizable = true;  // Typed values can be parameterized
+  action.metadata.Set("input_type", input_type);
+  action.metadata.Set("name", name);
+  action.metadata.Set("id", id);
+  
+  StoreAction(std::move(action));
+}
+
+void ActionRecorder::RecordNavigation(const GURL& url, bool is_user_initiated) {
+  // Only record user-initiated navigations
+  if (!is_user_initiated) {
+    return;
+  }
+  
+  if (!is_recording_) {
+    return;
+  }
+  
+  RecordedAction action = CreateBaseAction(ActionType::kNavigate);
+  action.url = url;
+  action.url_pattern = NormalizeUrl(url);
+  action.metadata.Set("user_initiated", true);
+  
+  StoreAction(std::move(action));
+}
+
+void ActionRecorder::RecordScroll(int delta_x, int delta_y,
+                                   int scroll_x, int scroll_y) {
+  if (!is_recording_ || !ShouldRecordForCurrentPage()) {
+    return;
+  }
+  
+  // Throttle scroll events to avoid recording every scroll tick
+  // Only record if enough time has passed since last scroll event
+  base::Time now = base::Time::Now();
+  if (!last_scroll_time_.is_null() &&
+      (now - last_scroll_time_) < kScrollThrottleInterval) {
+    return;  // Skip this scroll event (throttled)
+  }
+  last_scroll_time_ = now;
+  
+  // Only record significant scrolls (ignore tiny movements)
+  constexpr int kMinScrollDelta = 50;  // pixels
+  if (std::abs(delta_x) < kMinScrollDelta && 
+      std::abs(delta_y) < kMinScrollDelta) {
+    return;
+  }
+  
+  RecordedAction action = CreateBaseAction(ActionType::kScroll);
+  action.metadata.Set("delta_x", delta_x);
+  action.metadata.Set("delta_y", delta_y);
+  action.metadata.Set("scroll_x", scroll_x);
+  action.metadata.Set("scroll_y", scroll_y);
+  
+  StoreAction(std::move(action));
+}
+
+void ActionRecorder::RecordSubmit(const std::string& form_selector) {
+  if (!is_recording_ || !ShouldRecordForCurrentPage()) {
+    return;
+  }
+  
+  RecordedAction action = CreateBaseAction(ActionType::kSubmit);
+  action.selectors = GenerateSelectors(form_selector);
+  
+  StoreAction(std::move(action));
+}
+
+void ActionRecorder::RecordKeyPress(const std::string& key,
+                                     bool ctrl, bool alt, bool shift, bool meta) {
+  if (!is_recording_ || !ShouldRecordForCurrentPage()) {
+    return;
+  }
+  
+  RecordedAction action = CreateBaseAction(ActionType::kKeyPress);
+  action.value = key;
+  action.metadata.Set("ctrl", ctrl);
+  action.metadata.Set("alt", alt);
+  action.metadata.Set("shift", shift);
+  action.metadata.Set("meta", meta);
+  
+  StoreAction(std::move(action));
+}
+
+void ActionRecorder::DidFinishNavigation(
+    content::NavigationHandle* navigation_handle) {
+  if (!navigation_handle->IsInPrimaryMainFrame() ||
+      !navigation_handle->HasCommitted()) {
+    return;
+  }
+  
+  GURL new_url = navigation_handle->GetURL();
+  bool user_initiated = navigation_handle->HasUserGesture();
+  
+  // Update current URL
+  current_url_ = new_url;
+  
+  // Record if this was user-initiated
+  if (user_initiated && is_recording_) {
+    RecordNavigation(new_url, true);
+  }
+  
+  // Check if we should continue recording on this page
+  if (is_recording_ && !ShouldRecordForCurrentPage()) {
+    VLOG(1) << "[ghost_mode] Stopping recording - navigated to excluded page";
+    StopRecording();
+  }
+}
+
+void ActionRecorder::WebContentsDestroyed() {
+  StopRecording();
+}
+
+std::unique_ptr<ActionRecorder> CreateActionRecorder(
+    content::WebContents* web_contents,
+    PrefService* pref_service,
+    ActionStore* action_store) {
+  return std::make_unique<ActionRecorder>(web_contents, pref_service, action_store);
+}
+
+}  // namespace browseros::ghost_mode
