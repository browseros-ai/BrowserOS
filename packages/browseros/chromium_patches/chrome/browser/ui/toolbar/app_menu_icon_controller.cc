diff --git a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
index 93b96091cf8b8..58820e2807d5c 100644
--- a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
+++ b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
@@ -45,8 +45,7 @@ AppMenuIconController::Severity SeverityFromUpgradeLevel(
       case UpgradeDetector::UPGRADE_ANNOYANCE_NONE:
         break;
       case UpgradeDetector::UPGRADE_ANNOYANCE_VERY_LOW:
-        // kVeryLow is meaningless for stable channels.
-        return AppMenuIconController::Severity::NONE;
+        return AppMenuIconController::Severity::MEDIUM;
       case UpgradeDetector::UPGRADE_ANNOYANCE_LOW:
         return AppMenuIconController::Severity::LOW;
       case UpgradeDetector::UPGRADE_ANNOYANCE_ELEVATED:
