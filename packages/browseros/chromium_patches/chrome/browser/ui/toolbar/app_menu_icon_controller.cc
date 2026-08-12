diff --git a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
index aaa9903f665c3185a2529f43167ccd4f829ca956..765652a6f8a9038fa7fb6b88a186db724da3a7dd 100644
--- a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
+++ b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
@@ -50,8 +50,8 @@ AppMenuIconController::Severity SeverityFromUpgradeLevel(
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
