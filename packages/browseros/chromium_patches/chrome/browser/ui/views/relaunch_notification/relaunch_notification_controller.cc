diff --git a/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc b/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
index dc6c2295919b7ec8eb78d84345827cd5846a542c..6ca47f9dc25ece8b303588e3648f98821faddceb 100644
--- a/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
+++ b/chrome/browser/ui/views/relaunch_notification/relaunch_notification_controller.cc
@@ -114,11 +114,9 @@ void RelaunchNotificationController::OnUpgradeRecommended() {
 
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
