diff --git a/chrome/browser/ui/browser_window/public/browser_window_features.h b/chrome/browser/ui/browser_window/public/browser_window_features.h
index acb63225a9ab1..abfee23b28fad 100644
--- a/chrome/browser/ui/browser_window/public/browser_window_features.h
+++ b/chrome/browser/ui/browser_window/public/browser_window_features.h
@@ -90,6 +90,7 @@ class TabsFromOtherDevicesSidePanelCoordinator;
 class TabListBridge;
 class TabStripModel;
 class TabStripServiceFeature;
+class ThirdPartyLlmPanelCoordinator;
 class ToastController;
 class ToastService;
 class TranslateBubbleController;
@@ -286,6 +287,10 @@ class BrowserWindowFeatures {
     return extension_installed_watcher_.get();
   }
 
+  ThirdPartyLlmPanelCoordinator* third_party_llm_panel_coordinator() {
+    return third_party_llm_panel_coordinator_.get();
+  }
+
   glic::GlicIphController* glic_iph_controller() {
     return glic_iph_controller_.get();
   }
@@ -577,6 +582,10 @@ class BrowserWindowFeatures {
 
   std::unique_ptr<CommentsSidePanelCoordinator>
       comments_side_panel_coordinator_;
+
+  std::unique_ptr<ThirdPartyLlmPanelCoordinator>
+      third_party_llm_panel_coordinator_;
+
   raw_ptr<PinnedToolbarActions> pinned_toolbar_actions_ = nullptr;
 
   std::unique_ptr<ExtensionInstalledWatcher> extension_installed_watcher_;
