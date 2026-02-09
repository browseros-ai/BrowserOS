diff --git a/chrome/browser/ui/browser_window/internal/browser_window_features.cc b/chrome/browser/ui/browser_window/internal/browser_window_features.cc
index cecb7ad2b1147..901f87635a63e 100644
--- a/chrome/browser/ui/browser_window/internal/browser_window_features.cc
+++ b/chrome/browser/ui/browser_window/internal/browser_window_features.cc
@@ -103,6 +103,7 @@
 #include "chrome/browser/ui/views/profiles/profile_menu_coordinator.h"
 #include "chrome/browser/ui/views/send_tab_to_self/send_tab_to_self_toolbar_bubble_controller.h"
 #include "chrome/browser/ui/views/side_panel/bookmarks/bookmarks_side_panel_coordinator.h"
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/comments/comments_side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.h"
 #include "chrome/browser/ui/views/side_panel/history/history_side_panel_coordinator.h"
@@ -110,6 +111,8 @@
 #include "chrome/browser/ui/views/side_panel/reading_list/reading_list_side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_registry.h"
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
 #include "chrome/browser/ui/views/tabs/recent_activity_bubble_dialog_view.h"
 #include "chrome/browser/ui/views/tabs/tab_strip_action_container.h"
 #include "chrome/browser/ui/views/toolbar/chrome_labs/chrome_labs_coordinator.h"
@@ -381,6 +384,12 @@ void BrowserWindowFeatures::Init(BrowserWindowInterface* browser) {
   bookmarks_side_panel_coordinator_ =
       std::make_unique<BookmarksSidePanelCoordinator>();
 
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    third_party_llm_panel_coordinator_ =
+        std::make_unique<ThirdPartyLlmPanelCoordinator>(
+            profile, browser->GetTabStripModel());
+  }
+
   signin_view_controller_ = std::make_unique<SigninViewController>(
       browser, profile, tab_strip_model_);
 
@@ -603,6 +612,12 @@ void BrowserWindowFeatures::InitPostWindowConstruction(Browser* browser) {
   incognito_clear_browsing_data_dialog_coordinator_ =
       std::make_unique<IncognitoClearBrowsingDataDialogCoordinator>(profile);
 
+  // BrowserOS: Clash of GPTs coordinator
+  if (base::FeatureList::IsEnabled(features::kClashOfGpts)) {
+    clash_of_gpts_coordinator_ =
+        std::make_unique<ClashOfGptsCoordinator>(browser);
+  }
+
   if (browser_view) {
     color_provider_browser_helper_ =
         std::make_unique<ColorProviderBrowserHelper>(
