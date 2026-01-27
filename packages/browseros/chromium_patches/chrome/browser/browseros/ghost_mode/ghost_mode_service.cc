diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_service.cc b/chrome/browser/browseros/ghost_mode/ghost_mode_service.cc
new file mode 100644
index 0000000000000..f6a7b8c9d0e1f
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_service.cc
@@ -0,0 +1,289 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_service.h"
+
+#include <utility>
+
+#include "base/files/file_path.h"
+#include "base/json/json_writer.h"
+#include "base/logging.h"
+#include "base/task/thread_pool.h"
+#include "base/time/time.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h"
+#include "chrome/browser/profiles/profile.h"
+#include "components/prefs/pref_service.h"
+#include "content/public/browser/web_contents.h"
+
+namespace browseros::ghost_mode {
+
+namespace {
+
+// Pattern detection interval (5 minutes)
+constexpr base::TimeDelta kPatternDetectionInterval = base::Minutes(5);
+
+// Data cleanup interval (1 hour)
+constexpr base::TimeDelta kDataCleanupInterval = base::Hours(1);
+
+}  // namespace
+
+GhostModeService::GhostModeService(Profile* profile)
+    : profile_(profile),
+      pref_service_(profile->GetPrefs()) {
+  Initialize();
+}
+
+GhostModeService::~GhostModeService() = default;
+
+void GhostModeService::Initialize() {
+  if (!profile_ || !pref_service_) {
+    LOG(ERROR) << "GhostModeService: Invalid profile or pref service";
+    return;
+  }
+  
+  // Get data directory
+  base::FilePath data_dir = profile_->GetPath().AppendASCII("GhostMode");
+  
+  // Initialize action store
+  action_store_ = std::make_unique<ActionStore>(data_dir, pref_service_);
+  if (!action_store_->Initialize()) {
+    LOG(ERROR) << "GhostModeService: Failed to initialize action store";
+    return;
+  }
+  
+  // Initialize pattern detector
+  pattern_detector_ = std::make_unique<PatternDetector>(
+      action_store_.get(), pref_service_);
+  pattern_detector_->AddObserver(this);
+  
+  // Initialize suggestion controller
+  suggestion_controller_ = std::make_unique<SuggestionController>(
+      pref_service_);
+  suggestion_controller_->AddObserver(this);
+  
+  // Initialize workflow generator
+  workflow_generator_ = std::make_unique<WorkflowGenerator>();
+  
+  // Initialize ghost executor
+  ghost_executor_ = std::make_unique<GhostExecutor>();
+  
+  // Start periodic tasks if enabled
+  if (IsEnabled()) {
+    SchedulePatternDetection();
+    ScheduleDataCleanup();
+  }
+  
+  VLOG(1) << "GhostModeService initialized";
+}
+
+void GhostModeService::Shutdown() {
+  // Stop timers
+  pattern_detection_timer_.Stop();
+  data_cleanup_timer_.Stop();
+  
+  // Stop all recorders
+  recorders_.clear();
+  
+  // Remove observers
+  if (pattern_detector_) {
+    pattern_detector_->RemoveObserver(this);
+  }
+  if (suggestion_controller_) {
+    suggestion_controller_->RemoveObserver(this);
+  }
+  
+  VLOG(1) << "GhostModeService shutdown";
+}
+
+void GhostModeService::SetEnabled(bool enabled) {
+  pref_service_->SetBoolean(prefs::kGhostModeEnabled, enabled);
+  
+  if (enabled) {
+    SchedulePatternDetection();
+    ScheduleDataCleanup();
+  } else {
+    pattern_detection_timer_.Stop();
+    data_cleanup_timer_.Stop();
+    
+    // Stop all recorders
+    recorders_.clear();
+  }
+  
+  NotifyStateChanged(enabled);
+}
+
+bool GhostModeService::IsEnabled() const {
+  return pref_service_->GetBoolean(prefs::kGhostModeEnabled);
+}
+
+void GhostModeService::StartObserving(content::WebContents* web_contents) {
+  if (!IsEnabled() || !web_contents) {
+    return;
+  }
+  
+  // Check if already observing
+  if (recorders_.contains(web_contents)) {
+    return;
+  }
+  
+  // Create recorder for this WebContents
  auto recorder = std::make_unique<ActionRecorder>(
      web_contents, pref_service_, action_store_.get());
+  
+  VLOG(2) << "Started observing WebContents";
+}
+
+void GhostModeService::StopObserving(content::WebContents* web_contents) {
+  recorders_.erase(web_contents);
+  VLOG(2) << "Stopped observing WebContents";
+}
+
+void GhostModeService::DetectPatterns() {
+  if (!pattern_detector_) {
+    return;
+  }
+  
+  // Run detection on background thread
+  base::ThreadPool::PostTask(
+      FROM_HERE, {base::TaskPriority::USER_VISIBLE},
+      base::BindOnce(
+          [](PatternDetector* detector) { detector->DetectPatterns(); },
+          pattern_detector_.get()));
+}
+
+std::vector<ActionSequence> GhostModeService::GetDetectedPatterns() const {
+  if (!pattern_detector_) {
+    return {};
+  }
+  return pattern_detector_->GetDetectedPatterns();
+}
+
+std::string GhostModeService::ConvertPatternToWorkflow(
+    const std::string& pattern_id) {
+  if (!action_store_ || !workflow_generator_) {
+    return "";
+  }
+  
+  auto pattern = action_store_->GetPatternById(pattern_id);
+  if (!pattern) {
+    LOG(WARNING) << "Pattern not found: " << pattern_id;
+    return "";
+  }
+  
+  std::string json = workflow_generator_->Generate(*pattern);
+  NotifyWorkflowGenerated(json);
+  
+  return json;
+}
+
+void GhostModeService::ExecuteWorkflow(
+    const std::string& workflow_json,
+    GhostExecutor::CompletionCallback callback) {
+  if (!ghost_executor_) {
+    std::move(callback).Run(false, "Ghost executor not initialized");
+    return;
+  }
+  
+  ghost_executor_->Execute(workflow_json, std::move(callback));
+}
+
+void GhostModeService::PauseExecution() {
+  if (ghost_executor_) {
+    ghost_executor_->Pause();
+  }
+}
+
+void GhostModeService::ResumeExecution() {
+  if (ghost_executor_) {
+    ghost_executor_->Resume();
+  }
+}
+
+GhostModeService::Stats GhostModeService::GetStats() const {
+  Stats stats;
+  if (action_store_) {
+    stats.total_actions = action_store_->GetTotalActionCount();
+    stats.total_patterns = action_store_->GetPatternCount();
+    stats.total_workflows = action_store_->GetWorkflowCount();
+  }
+  return stats;
+}
+
+void GhostModeService::ClearAllData() {
+  if (action_store_) {
+    action_store_->DeleteAllActions();
+    action_store_->DeleteAllPatterns();
+  }
+  NotifyStatsUpdated();
+}
+
+void GhostModeService::AddExcludedDomain(const std::string& domain) {
+  // Implementation via prefs
+  // Similar to browseros_prefs_page pattern
+}
+
+void GhostModeService::RemoveExcludedDomain(const std::string& domain) {
+  // Implementation via prefs
+}
+
+std::vector<std::string> GhostModeService::GetExcludedDomains() const {
+  return {};  // Load from prefs
+}
+
+void GhostModeService::DeletePattern(const std::string& pattern_id) {
+  if (action_store_) {
+    action_store_->DeletePattern(pattern_id);
+  }
+}
+
+void GhostModeService::DismissPattern(const std::string& pattern_id) {
+  if (suggestion_controller_) {
+    suggestion_controller_->DismissPermanently(pattern_id);
+  }
+}
+
+void GhostModeService::AddObserver(GhostModeServiceObserver* observer) {
+  observers_.AddObserver(observer);
+}
+
+void GhostModeService::RemoveObserver(GhostModeServiceObserver* observer) {
+  observers_.RemoveObserver(observer);
+}
+
+void GhostModeService::OnPatternDetected(const ActionSequence& pattern) {
+  NotifyPatternDetected(pattern);
+  
+  // Show suggestion if appropriate
+  if (suggestion_controller_) {
+    suggestion_controller_->MaybeSuggest(pattern);
+  }
+}
+
+void GhostModeService::OnDetectionComplete(int patterns_found) {
+  VLOG(1) << "Pattern detection complete: " << patterns_found << " patterns";
+  NotifyStatsUpdated();
+}
+
+void GhostModeService::OnSuggestionAccepted(const ActionSequence& pattern) {
+  ConvertPatternToWorkflow(pattern.id);
+}
+
+void GhostModeService::OnSuggestionDismissed(const std::string& pattern_id) {
+  DismissPattern(pattern_id);
+}
+
+void GhostModeService::OnSuggestionDeferred(const std::string& pattern_id) {
+  // Will ask again later
+}
+
+void GhostModeService::SchedulePatternDetection() {
+  pattern_detection_timer_.Start(
+      FROM_HERE, kPatternDetectionInterval,
+      base::BindRepeating(&GhostModeService::OnPatternDetectionTimer,
+                          base::Unretained(this)));
+}
+
+void GhostModeService::OnPatternDetectionTimer() {
+  DetectPatterns();
+}
+
+void GhostModeService::ScheduleDataCleanup() {
+  data_cleanup_timer_.Start(
+      FROM_HERE, kDataCleanupInterval,
+      base::BindRepeating(&GhostModeService::OnDataCleanupTimer,
+                          base::Unretained(this)));
+}
+
+void GhostModeService::OnDataCleanupTimer() {
+  if (action_store_) {
+    action_store_->CleanupOldData();
+  }
+}
+
+void GhostModeService::NotifyStateChanged(bool enabled) {
+  for (auto& observer : observers_) {
+    observer.OnGhostModeStateChanged(enabled);
+  }
+}
+
+void GhostModeService::NotifyPatternDetected(const ActionSequence& pattern) {
+  for (auto& observer : observers_) {
+    observer.OnPatternDetected(pattern);
+  }
+}
+
+void GhostModeService::NotifyWorkflowGenerated(const std::string& json) {
+  for (auto& observer : observers_) {
+    observer.OnWorkflowGenerated(json);
+  }
+}
+
+void GhostModeService::NotifyStatsUpdated() {
+  auto stats = GetStats();
+  for (auto& observer : observers_) {
+    observer.OnStatsUpdated(stats.total_actions, stats.total_patterns,
+                            stats.total_workflows);
+  }
+}
+
+}  // namespace browseros::ghost_mode
