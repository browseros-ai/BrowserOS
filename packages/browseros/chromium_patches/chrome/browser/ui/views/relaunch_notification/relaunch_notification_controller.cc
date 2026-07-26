diff --git a/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc b/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
index 35e2b16b88a58..ddcfc5176d0cf 100644
--- a/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
+++ b/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
@@ -115,11 +115,9 @@ void RelaunchNotificationController::OnUpgradeRecommended() {
 
   switch (current_level) {
     case UpgradeDetector::UPGRADE_ANNOYANCE_NONE:
-    case UpgradeDetector::UPGRADE_ANNOYANCE_VERY_LOW:
-      // While it's unexpected that the level could move back down, it's not a
-      // challenge to do the right thing.
       CloseRelaunchNotification();
       break;
+    case UpgradeDetector::UPGRADE_ANNOYANCE_VERY_LOW:
     case UpgradeDetector::UPGRADE_ANNOYANCE_LOW:
     case UpgradeDetector::UPGRADE_ANNOYANCE_ELEVATED:
     case UpgradeDetector::UPGRADE_ANNOYANCE_GRACE:
