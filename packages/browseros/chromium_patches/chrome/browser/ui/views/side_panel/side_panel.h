diff --git a/chrome/browser/ui/views/side_panel/side_panel.h b/chrome/browser/ui/views/side_panel/side_panel.h
index 340d36f59e278..32dac5e143fbd 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.h
+++ b/chrome/browser/ui/views/side_panel/side_panel.h
@@ -170,6 +170,9 @@ class SidePanel : public views::AccessiblePaneView,
 
   bool animations_disabled_ = false;
 
+  // BrowserOS: flag to control animations
+  bool animations_disabled_browseros_ = true;
+
   // Starting bounds for the side panel content if kOpenWithContentTransition
   // animation is shown.
   std::optional<gfx::Rect> content_starting_bounds_;
