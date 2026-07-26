diff --git a/chrome/browser/ui/views/side_panel/side_panel.h b/chrome/browser/ui/views/side_panel/side_panel.h
index 4946a573e264e..da735d4bf11f6 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.h
+++ b/chrome/browser/ui/views/side_panel/side_panel.h
@@ -164,6 +164,9 @@ class SidePanel : public views::AccessiblePaneView,
 
   bool animations_disabled_ = false;
 
+  // BrowserOS: flag to control animations
+  bool animations_disabled_browseros_ = true;
+
   // Starting bounds for the side panel content if kOpenWithContentTransition
   // animation is shown.
   std::optional<gfx::Rect> content_starting_bounds_;
