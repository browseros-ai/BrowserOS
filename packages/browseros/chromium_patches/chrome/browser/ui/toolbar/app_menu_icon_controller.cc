diff --git a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
index 62486948bce84..fbe9b650fd29e 100644
--- a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
+++ b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
@@ -45,8 +45,8 @@ AppMenuIconController::Severity SeverityFromUpgradeLevel(
       case UpgradeDetector::UPGRADE_ANNOYANCE_NONE:
         break;
       case UpgradeDetector::UPGRADE_ANNOYANCE_VERY_LOW:
-        // kVeryLow is meaningless for stable channels.
-        return AppMenuIconController::Severity::kNone;
+        // BrowserOS: show update indicator sooner
+        return AppMenuIconController::Severity::kMedium;
       case UpgradeDetector::UPGRADE_ANNOYANCE_LOW:
         return AppMenuIconController::Severity::kLow;
       case UpgradeDetector::UPGRADE_ANNOYANCE_ELEVATED:
