diff --git a/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc b/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
index 8b4056c06a84e..1cef76459e91a 100644
--- a/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
+++ b/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
@@ -108,11 +108,9 @@ void RelaunchNotificationController::OnUpgradeRecommended() {
 
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
