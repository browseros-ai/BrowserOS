diff --git a/chrome/browser/ui/browser_command_controller.cc b/chrome/browser/ui/browser_command_controller.cc
index 52f78a5a465840db35f411c1890090ee9bf69fd5..68b26e420cdb976e65592e7c40af69768b93b6c4 100644
--- a/chrome/browser/ui/browser_command_controller.cc
+++ b/chrome/browser/ui/browser_command_controller.cc
@@ -65,6 +65,7 @@
 #include "chrome/browser/ui/tabs/tab_strip_user_gesture_details.h"
 #include "chrome/browser/ui/toolbar/chrome_labs/chrome_labs_utils.h"
 #include "chrome/browser/ui/ui_features.h"
+#include "chrome/browser/ui/views/frame/browser_view.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_entry_id.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_enums.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_ui.h"
@@ -939,6 +940,12 @@ bool BrowserCommandController::ExecuteCommandWithDisposition(
         coordinator->Show();
       }
       break;
+    case IDC_TOGGLE_AGENT_SPLIT:
+      if (auto* browser_view =
+              BrowserView::GetBrowserViewForBrowser(browser_)) {
+        browser_view->ToggleAgentSplit();
+      }
+      break;
     case IDC_SHOW_APP_MENU:
       base::RecordAction(base::UserMetricsAction("Accel_Show_App_Menu"));
       ShowAppMenu(browser_);
@@ -1583,6 +1590,7 @@ void BrowserCommandController::InitCommandState() {
                                         base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel));
   command_updater_.UpdateCommandEnabled(IDC_OPEN_CLASH_OF_GPTS,
                                         base::FeatureList::IsEnabled(features::kClashOfGpts));
+  command_updater_.UpdateCommandEnabled(IDC_TOGGLE_AGENT_SPLIT, normal_window);
 
   if (browser_->is_type_normal()) {
     // Reading list commands.
