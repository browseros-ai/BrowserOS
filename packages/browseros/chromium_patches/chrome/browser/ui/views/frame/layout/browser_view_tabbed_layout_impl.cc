diff --git a/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc b/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
index 2ad3b0006ad01..71dd1c47498b1 100644
--- a/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
+++ b/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
@@ -47,7 +47,7 @@ constexpr int kLoadingBarOffset =
     kLoadingBarHeight - views::Separator::kThickness;
 
 // Minimum area next to caption buttons to use as a grab handle.
-constexpr int kVerticalTabsGrabHandleSize = 40;
+constexpr int kVerticalTabsGrabHandleSize = 5;
 
 // Maximum portion of the window a "size-restricted" contents-height side panel
 // can take up. This is not the only limit on side panel size.
