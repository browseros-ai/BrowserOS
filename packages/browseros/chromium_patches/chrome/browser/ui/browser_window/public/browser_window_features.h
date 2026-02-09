diff --git a/chrome/browser/ui/browser_window/public/browser_window_features.h b/chrome/browser/ui/browser_window/public/browser_window_features.h
index f1fc1743c6b4b..1d36a9bde9e7c 100644
--- a/chrome/browser/ui/browser_window/public/browser_window_features.h
+++ b/chrome/browser/ui/browser_window/public/browser_window_features.h
@@ -43,6 +43,7 @@ class BrowserUserEducationInterface;
 class BrowserView;
 class BrowserWindowInterface;
 class ChromeLabsCoordinator;
+class ClashOfGptsCoordinator;
 class ColorProviderBrowserHelper;
 class LocationBar;
 class CommentsSidePanelCoordinator;
@@ -86,6 +87,7 @@ class TabSearchToolbarButtonController;
 class TabListBridge;
 class TabStripModel;
 class TabStripServiceFeature;
+class ThirdPartyLlmPanelCoordinator;
 class ToastController;
 class ToastService;
 class TranslateBubbleController;
@@ -277,6 +279,14 @@ class BrowserWindowFeatures {
     return extension_installed_watcher_.get();
   }
 
+  ThirdPartyLlmPanelCoordinator* third_party_llm_panel_coordinator() {
+    return third_party_llm_panel_coordinator_.get();
+  }
+
+  ClashOfGptsCoordinator* clash_of_gpts_coordinator() {
+    return clash_of_gpts_coordinator_.get();
+  }
+
 #if BUILDFLAG(ENABLE_GLIC)
   glic::GlicLegacySidePanelCoordinator* glic_side_panel_coordinator() {
     return glic_side_panel_coordinator_.get();
@@ -590,6 +600,11 @@ class BrowserWindowFeatures {
   std::unique_ptr<CommentsSidePanelCoordinator>
       comments_side_panel_coordinator_;
 
+  std::unique_ptr<ThirdPartyLlmPanelCoordinator>
+      third_party_llm_panel_coordinator_;
+
+  std::unique_ptr<ClashOfGptsCoordinator> clash_of_gpts_coordinator_;
+
   std::unique_ptr<PinnedToolbarActionsController>
       pinned_toolbar_actions_controller_;
 
