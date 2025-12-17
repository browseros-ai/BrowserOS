diff --git a/chrome/browser/upgrade_detector/upgrade_detector_impl.cc b/chrome/browser/upgrade_detector/upgrade_detector_impl.cc
index 15ca9a708125d..30ab918d42479 100644
--- a/chrome/browser/upgrade_detector/upgrade_detector_impl.cc
+++ b/chrome/browser/upgrade_detector/upgrade_detector_impl.cc
@@ -48,7 +48,7 @@
 namespace {
 
 // The default thresholds for reaching annoyance levels.
-constexpr auto kDefaultVeryLowThreshold = base::Hours(1);
+constexpr auto kDefaultVeryLowThreshold = base::Minutes(1);
 constexpr auto kDefaultLowThreshold = base::Days(2);
 constexpr auto kDefaultElevatedThreshold = base::Days(4);
 constexpr auto kDefaultHighThreshold = base::Days(7);
@@ -275,6 +275,8 @@ void UpgradeDetectorImpl::DetectOutdatedInstall() {
 void UpgradeDetectorImpl::UpgradeDetected(UpgradeAvailable upgrade_available) {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
 
+  VLOG(1) << "UpgradeDetector: UpgradeDetected called, type=" << upgrade_available;
+
   set_upgrade_available(upgrade_available);
   set_critical_update_acknowledged(false);
 
@@ -327,6 +329,10 @@ void UpgradeDetectorImpl::NotifyOnUpgradeWithTimePassed(
       next_delay = *(it - 1) - time_passed;
   }
 
+  VLOG(1) << "UpgradeDetector: time_passed=" << time_passed.InSeconds()
+          << "s, stage=" << new_stage << " (was " << last_stage
+          << "), next_delay=" << next_delay.InSeconds() << "s";
+
   set_upgrade_notification_stage(new_stage);
   if (!next_delay.is_zero()) {
     // Schedule the next wakeup in 20 minutes or when the next change to the
@@ -529,6 +535,9 @@ base::Time UpgradeDetectorImpl::GetAnnoyanceLevelDeadline(
 void UpgradeDetectorImpl::OnUpdate(const BuildState* build_state) {
   DCHECK_CALLED_ON_VALID_SEQUENCE(sequence_checker_);
 
+  VLOG(1) << "UpgradeDetector: OnUpdate called, type="
+          << static_cast<int>(build_state->update_type());
+
   if (build_state->update_type() == BuildState::UpdateType::kNone) {
     // An update was available, but seemingly no longer is. Perhaps an update
     // was followed by a rollback. Back off if nothing more important was
