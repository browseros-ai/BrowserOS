diff --git a/chrome/browser/ui/browser_window/internal/browser_window_features.cc b/chrome/browser/ui/browser_window/internal/browser_window_features.cc
index 9bad9b02b394d..277ed52674746 100644
--- a/chrome/browser/ui/browser_window/internal/browser_window_features.cc
+++ b/chrome/browser/ui/browser_window/internal/browser_window_features.cc
@@ -134,6 +134,7 @@
 #include "chrome/browser/ui/views/side_panel/reading_list/reading_list_side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/tabs_from_other_devices/tabs_from_other_devices_side_panel_coordinator.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
 #include "chrome/browser/ui/views/tabs/groups/recent_activity_bubble_dialog_view.h"
 #include "chrome/browser/ui/views/tabs/projects/projects_panel_utils.h"
 #include "chrome/browser/ui/views/tabs/tab_strip_action_container.h"
@@ -430,6 +431,12 @@ void BrowserWindowFeatures::Init(BrowserWindowInterface* browser) {
       GetUserDataFactory().CreateInstance<BookmarksSidePanelCoordinator>(
           *browser, *browser);
 
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    third_party_llm_panel_coordinator_ =
+        std::make_unique<ThirdPartyLlmPanelCoordinator>(
+            profile, browser->GetTabStripModel());
+  }
+
   signin_view_controller_ = std::make_unique<SigninViewController>(
       browser, profile, tab_strip_model_);
 
