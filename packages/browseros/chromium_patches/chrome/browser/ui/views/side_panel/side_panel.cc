diff --git a/chrome/browser/ui/views/side_panel/side_panel.cc b/chrome/browser/ui/views/side_panel/side_panel.cc
index 90f261de2344dd9674f6b10599b80ffd6a0eb951..d13fe9aa60d34f57448828ac2514735e8a396ed0 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.cc
+++ b/chrome/browser/ui/views/side_panel/side_panel.cc
@@ -716,8 +716,10 @@ double SidePanel::GetAnimationValueFor(BrowserAnimationSequence which) const {
 }
 
 bool SidePanel::ShouldShowAnimation() const {
+  // BrowserOS: animations_disabled_browseros_ used to control animation
   bool should_show_animations =
-      gfx::Animation::ShouldRenderRichAnimation() && !animations_disabled_;
+      gfx::Animation::ShouldRenderRichAnimation() && !animations_disabled_ &&
+      animations_disabled_browseros_;
   return should_show_animations;
 }
 
