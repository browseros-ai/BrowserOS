diff --git a/chrome/browser/ui/views/side_panel/side_panel.h b/chrome/browser/ui/views/side_panel/side_panel.h
index 3db88e2d54ca1c8bed014ca5848afaf296a7e061..e028a3559343b7ef50660cf87f51a302588e06f5 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.h
+++ b/chrome/browser/ui/views/side_panel/side_panel.h
@@ -163,6 +163,9 @@ class SidePanel : public views::AccessiblePaneView,
 
   bool animations_disabled_ = false;
 
+  // BrowserOS: flag to control animations
+  bool animations_disabled_browseros_ = true;
+
   // Starting bounds for the side panel content if kOpenWithContentTransition
   // animation is shown.
   std::optional<gfx::Rect> content_starting_bounds_;
