diff --git a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
index c944c21557711e0026b2cfb21f6fa672a4a5cb49..8eeae9182b24510e3832ea092e025d4a47739954 100644
--- a/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
+++ b/chrome/browser/ui/toolbar/app_menu_icon_controller.cc
@@ -57,8 +57,8 @@ AppMenuIconController::Severity SeverityFromUpgradeLevel(
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
