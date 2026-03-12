diff --git a/chrome/browser/upgrade_detector/upgrade_detector_impl.cc b/chrome/browser/upgrade_detector/upgrade_detector_impl.cc
index 07b7e0fcf5119..68c429d702873 100644
--- a/chrome/browser/upgrade_detector/upgrade_detector_impl.cc
+++ b/chrome/browser/upgrade_detector/upgrade_detector_impl.cc
@@ -49,11 +49,13 @@
 namespace {
 
 // The default thresholds for reaching annoyance levels.
-constexpr auto kDefaultVeryLowThreshold = base::Hours(1);
-constexpr auto kDefaultLowThreshold = base::Days(2);
-constexpr auto kDefaultElevatedThreshold = base::Days(4);
-constexpr auto kDefaultHighThreshold = base::Days(7);
-constexpr auto kDefaultGraceThreshold = kDefaultHighThreshold - base::Hours(1);
+// These may be unused when ENABLE_SPARKLE is true (Sparkle uses its own thresholds).
+[[maybe_unused]] constexpr auto kDefaultVeryLowThreshold = base::Hours(1);
+[[maybe_unused]] constexpr auto kDefaultLowThreshold = base::Days(2);
+[[maybe_unused]] constexpr auto kDefaultElevatedThreshold = base::Days(4);
+[[maybe_unused]] constexpr auto kDefaultHighThreshold = base::Days(7);
+[[maybe_unused]] constexpr auto kDefaultGraceThreshold =
+    kDefaultHighThreshold - base::Hours(1);
 
 // How long to wait (each cycle) before checking which severity level we should
 // be at. Once we reach the highest severity, the timer will stop.
@@ -69,7 +71,10 @@ constexpr auto kOutdatedBuildDetectorPeriod = base::Days(1);
 constexpr auto kOutdatedBuildAge = base::Days(7) * 8;
 
 bool ShouldDetectOutdatedBuilds() {
-#if BUILDFLAG(ENABLE_UPDATE_NOTIFICATIONS) && !BUILDFLAG(IS_CHROMEOS)
+#if BUILDFLAG(ENABLE_SPARKLE)
+  // Sparkle handles its own updates, no need for outdated build detection.
+  return false;
+#elif BUILDFLAG(ENABLE_UPDATE_NOTIFICATIONS) && !BUILDFLAG(IS_CHROMEOS)
   // Don't show the bubble if we have a brand code that is NOT organic
   std::string brand;
   if (google_brand::GetBrand(&brand) && !google_brand::IsOrganic(brand)) {
@@ -163,6 +168,16 @@ void UpgradeDetectorImpl::CalculateThresholds() {
 void UpgradeDetectorImpl::DoCalculateThresholds() {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
 
+#if BUILDFLAG(ENABLE_SPARKLE)
+  // Sparkle notifies us when updates are ready to install.
+  // Zero thresholds so the badge appears immediately on the same call stack
+  // as NotifyUpgradeReady() — no timer delay needed.
+  stages_[kStagesIndexVeryLow] = base::TimeDelta();
+  stages_[kStagesIndexLow] = base::TimeDelta();
+  stages_[kStagesIndexElevated] = base::TimeDelta();
+  stages_[kStagesIndexGrace] = base::TimeDelta();
+  stages_[kStagesIndexHigh] = base::TimeDelta();
+#else   // !BUILDFLAG(ENABLE_SPARKLE)
   base::TimeDelta notification_period = GetRelaunchNotificationPeriod();
   const std::optional<RelaunchWindow> relaunch_window =
       GetRelaunchWindowPolicyValue();
@@ -216,6 +231,7 @@ void UpgradeDetectorImpl::DoCalculateThresholds() {
     for (auto& stage : stages_)
       stage /= scale_factor;
   }
+#endif  // BUILDFLAG(ENABLE_SPARKLE)
 }
 
 void UpgradeDetectorImpl::StartOutdatedBuildDetector() {
@@ -281,6 +297,8 @@ void UpgradeDetectorImpl::DetectOutdatedInstall() {
 void UpgradeDetectorImpl::UpgradeDetected(UpgradeAvailable upgrade_available) {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
 
+  VLOG(1) << "UpgradeDetector: UpgradeDetected called, type=" << upgrade_available;
+
   set_upgrade_available(upgrade_available);
   set_critical_update_acknowledged(false);
 
@@ -333,6 +351,10 @@ void UpgradeDetectorImpl::NotifyOnUpgradeWithTimePassed(
       next_delay = *(it - 1) - time_passed;
   }
 
+  VLOG(1) << "UpgradeDetector: time_passed=" << time_passed.InSeconds()
+          << "s, stage=" << new_stage << " (was " << last_stage
+          << "), next_delay=" << next_delay.InSeconds() << "s";
+
   set_upgrade_notification_stage(new_stage);
   if (!next_delay.is_zero()) {
     // Schedule the next wakeup in 20 minutes or when the next change to the
@@ -360,7 +382,6 @@ void UpgradeDetectorImpl::NotifyOnUpgradeWithTimePassed(
 base::TimeDelta UpgradeDetectorImpl::GetThresholdForLevel(
     UpgradeNotificationAnnoyanceLevel level) {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
-  DCHECK(!stages_[0].is_zero());
   return stages_[AnnoyanceLevelToStagesIndex(level)];
 }
 
@@ -491,7 +512,10 @@ void UpgradeDetectorImpl::Init() {
 
   auto* const build_state = g_browser_process->GetBuildState();
   build_state->AddObserver(this);
+#if !BUILDFLAG(ENABLE_SPARKLE)
+  // Sparkle handles version checking via appcast, no need to poll file system.
   installed_version_poller_.emplace(build_state);
+#endif  // !BUILDFLAG(ENABLE_SPARKLE)
 #endif  // BUILDFLAG(ENABLE_UPDATE_NOTIFICATIONS)
 }
 
@@ -535,6 +559,9 @@ base::Time UpgradeDetectorImpl::GetAnnoyanceLevelDeadline(
 void UpgradeDetectorImpl::OnUpdate(const BuildState* build_state) {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
 
+  VLOG(1) << "UpgradeDetector: OnUpdate called, type="
+          << static_cast<int>(build_state->update_type());
+
   if (build_state->update_type() == BuildState::UpdateType::kNone) {
     // An update was available, but seemingly no longer is. Perhaps an update
     // was followed by a rollback. Back off if nothing more important was
