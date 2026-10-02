diff --git a/chrome/browser/ui/browser_window/public/browser_window_features.h b/chrome/browser/ui/browser_window/public/browser_window_features.h
index 309eced7f092e28c5c9f7119696860068d626ed9..f15b13500665faf520cffa05ffb9e9315e1228ef 100644
--- a/chrome/browser/ui/browser_window/public/browser_window_features.h
+++ b/chrome/browser/ui/browser_window/public/browser_window_features.h
@@ -112,6 +112,7 @@ class TabMenuModelDelegate;
 class TabStripModel;
 class TabStripServiceFeature;
 class TabsFromOtherDevicesSidePanelCoordinator;
+class ThirdPartyLlmPanelCoordinator;
 class ToastService;
 class TranslateBubbleController;
 class UIControllerFactory;
@@ -330,6 +331,12 @@ class BrowserWindowFeatures {
     return pinned_toolbar_actions_;
   }
 
+  // BrowserOS owns this coordinator for the lifetime of the window. It is
+  // created in Init() only when the LLM panel feature is enabled.
+  ThirdPartyLlmPanelCoordinator* third_party_llm_panel_coordinator() {
+    return third_party_llm_panel_coordinator_.get();
+  }
+
   static ui::UserDataFactoryWithOwner<BrowserWindowInterface>&
   GetUserDataFactoryForTesting();
 
@@ -486,6 +493,8 @@ class BrowserWindowFeatures {
 
   std::unique_ptr<TabsFromOtherDevicesSidePanelCoordinator>
       tabs_from_other_devices_side_panel_coordinator_;
+  std::unique_ptr<ThirdPartyLlmPanelCoordinator>
+      third_party_llm_panel_coordinator_;
   std::unique_ptr<ToastService> toast_service_;
   std::unique_ptr<TranslateBubbleController> translate_bubble_controller_;
   std::unique_ptr<UpgradeNotificationController>
