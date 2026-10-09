diff --git a/chrome/browser/ui/views/side_panel/side_panel.h b/chrome/browser/ui/views/side_panel/side_panel.h
index de391b015c1fa483b38a0e85a74629c211021261..cddfe03518b8012f83cbc1971aa7fa84a7548c4c 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.h
+++ b/chrome/browser/ui/views/side_panel/side_panel.h
@@ -162,6 +162,9 @@ class SidePanel : public views::AccessiblePaneView,
 
   bool animations_disabled_ = false;
 
+  // BrowserOS: flag to control animations
+  bool animations_disabled_browseros_ = true;
+
   // Starting bounds for the side panel content if kOpenWithContentTransition
   // animation is shown.
   std::optional<gfx::Rect> content_starting_bounds_;
