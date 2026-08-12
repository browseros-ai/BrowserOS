diff --git a/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc b/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
index 67fdce98388217ee031a9f1eb1c187a813a823da..a5655f6161ba651362c58e6388c9c6701c2a154b 100644
--- a/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
+++ b/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
@@ -54,7 +54,7 @@
 namespace {
 
 // Minimum area next to caption buttons to use as a grab handle.
-constexpr int kVerticalTabsGrabHandleSize = 40;
+constexpr int kVerticalTabsGrabHandleSize = 5;
 
 // Maximum portion of the window a "size-restricted" contents-height side panel
 // can take up. This is not the only limit on side panel size.
