diff --git a/chrome/browser/ui/browser_window/internal/browser_window_features.cc b/chrome/browser/ui/browser_window/internal/browser_window_features.cc
index c2d47effffd2298e037a090dc3ad22ecd9315dbd..34ef13a8cc02a445a17575fc826f1324cd8cc2ad 100644
--- a/chrome/browser/ui/browser_window/internal/browser_window_features.cc
+++ b/chrome/browser/ui/browser_window/internal/browser_window_features.cc
@@ -149,6 +149,7 @@
 #include "chrome/browser/ui/views/side_panel/reading_list/reading_list_side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/tabs_from_other_devices/tabs_from_other_devices_side_panel_coordinator.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
 #include "chrome/browser/ui/views/tabs/groups/recent_activity_bubble_dialog_view.h"
 #include "chrome/browser/ui/views/tabs/projects/projects_panel_utils.h"
 #include "chrome/browser/ui/views/tabs/tab_strip_action_container.h"
@@ -520,6 +521,12 @@ void BrowserWindowFeatures::Init(BrowserWindowInterface* browser) {
                                                                    profile);
   }
 
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    third_party_llm_panel_coordinator_ =
+        std::make_unique<ThirdPartyLlmPanelCoordinator>(
+            profile, browser->GetTabStripModel());
+  }
+
   translate_bubble_controller_ =
       GetUserDataFactory().CreateInstance<TranslateBubbleController>(
           *browser, browser, browser_actions_->root_action_item());
@@ -622,7 +629,7 @@ void BrowserWindowFeatures::Init(BrowserWindowInterface* browser) {
   //   CloseButtonController depends on VerticalTabStripStateController.
   //   CloseButtonController depends on ImmersiveModeController.
   // TODO(crbug.com/481268779): Pass these dependencies explicitly.
-  if (contextual_tasks::IsContextualTasksUIEnabled()) {
+  if (base::FeatureList::IsEnabled(contextual_tasks::kContextualTasks)) {
     contextual_tasks_browser_controller_ =
         GetUserDataFactory()
             .CreateInstance<contextual_tasks::ContextualTasksBrowserController>(
