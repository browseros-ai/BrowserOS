diff --git a/chrome/browser/ui/ui_features.h b/chrome/browser/ui/ui_features.h
index 2bedb1e230459..f779499960910 100644
--- a/chrome/browser/ui/ui_features.h
+++ b/chrome/browser/ui/ui_features.h
@@ -161,6 +161,10 @@ BASE_DECLARE_FEATURE(kPopupBrowserUseNewLayout);
 
 BASE_DECLARE_FEATURE(kTabbedBrowserUseNewLayout);
 
+// BrowserOS: feature declarations
+BASE_DECLARE_FEATURE(kThirdPartyLlmPanel);
+BASE_DECLARE_FEATURE(kClashOfGpts);
+
 BASE_DECLARE_FEATURE(kTabDuplicateMetrics);
 
 BASE_DECLARE_FEATURE(kTabGroupsCollapseFreezing);
