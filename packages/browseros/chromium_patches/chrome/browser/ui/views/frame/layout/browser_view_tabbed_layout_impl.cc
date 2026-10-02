diff --git a/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc b/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
index 29dfbde914c55ad1453a1c70998c9299b49c1132..9285a73f26b536b56fd328e901b98d537e4ee859 100644
--- a/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
+++ b/chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc
@@ -62,7 +62,7 @@
 namespace {
 
 // Minimum area next to caption buttons to use as a grab handle.
-constexpr int kVerticalTabsGrabHandleSize = 40;
+constexpr int kVerticalTabsGrabHandleSize = 5;
 
 // Maximum portion of the window a "size-restricted" contents-height side panel
 // can take up. This is not the only limit on side panel size.
