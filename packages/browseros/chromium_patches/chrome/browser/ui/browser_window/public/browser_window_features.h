diff --git a/chrome/browser/ui/browser_window/public/browser_window_features.h b/chrome/browser/ui/browser_window/public/browser_window_features.h
index daa62a7361df8bc13998ff7f90839421685c615b..263546e78e902c221fa800ed3a6febd1027720bb 100644
--- a/chrome/browser/ui/browser_window/public/browser_window_features.h
+++ b/chrome/browser/ui/browser_window/public/browser_window_features.h
@@ -85,6 +85,7 @@ class TabMenuModelDelegate;
 class TabStripModel;
 class TabStripServiceFeature;
 class TabsFromOtherDevicesSidePanelCoordinator;
+class ThirdPartyLlmPanelCoordinator;
 class ToastController;
 class ToastService;
 class TranslateBubbleController;
@@ -500,6 +501,10 @@ class BrowserWindowFeatures {
     return tabs_from_other_devices_side_panel_coordinator_.get();
   }
 
+  ThirdPartyLlmPanelCoordinator* third_party_llm_panel_coordinator() {
+    return third_party_llm_panel_coordinator_.get();
+  }
+
   // Returns a pointer to the ToastController for the browser window. This can
   // return nullptr for non-normal browser windows because toasts are not
   // supported for those cases.
@@ -670,6 +675,8 @@ class BrowserWindowFeatures {
 
   std::unique_ptr<TabsFromOtherDevicesSidePanelCoordinator>
       tabs_from_other_devices_side_panel_coordinator_;
+  std::unique_ptr<ThirdPartyLlmPanelCoordinator>
+      third_party_llm_panel_coordinator_;
   std::unique_ptr<ToastService> toast_service_;
   std::unique_ptr<TranslateBubbleController> translate_bubble_controller_;
   std::unique_ptr<UpgradeNotificationController>
