diff --git a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
index c920f7ef7d6f2..b371c96ddcbf1 100644
--- a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
+++ b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
@@ -44,8 +44,8 @@ AppMenuIconController::Severity SeverityFromUpgradeLevel(
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
