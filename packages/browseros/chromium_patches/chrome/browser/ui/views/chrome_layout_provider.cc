diff --git a/chrome/browser/ui/views/chrome_layout_provider.cc b/chrome/browser/ui/views/chrome_layout_provider.cc
index 77be8f6e709d3..5f0dd9b1960d7 100644
--- a/chrome/browser/ui/views/chrome_layout_provider.cc
+++ b/chrome/browser/ui/views/chrome_layout_provider.cc
@@ -167,9 +167,10 @@ int ChromeLayoutProvider::GetDistanceMetric(int metric) const {
       // top and bottom should be 8dp.
       // The new refreshed button height is 20 + (2 * 6) = 32dp.
       // Therefore, the total infobar height is 32dp + 2 * 12.
+      // BrowserOS: reduced padding for non-refresh infobars (2*3 instead of 2*8)
       return base::FeatureList::IsEnabled(features::kInfobarRefresh)
                  ? 32 + 2 * 12
-                 : 36 + 2 * 8;
+                 : 36 + 2 * 3;
     case DISTANCE_PERMISSION_PROMPT_HORIZONTAL_ICON_LABEL_PADDING:
       return 8;
     case DISTANCE_RICH_HOVER_BUTTON_ICON_HORIZONTAL:
