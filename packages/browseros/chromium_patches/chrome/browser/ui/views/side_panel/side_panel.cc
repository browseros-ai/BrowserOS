diff --git a/chrome/browser/ui/views/side_panel/side_panel.cc b/chrome/browser/ui/views/side_panel/side_panel.cc
index b341cbd160ba3..8a637ab8bd2f6 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.cc
+++ b/chrome/browser/ui/views/side_panel/side_panel.cc
@@ -967,8 +967,10 @@ double SidePanel::GetAnimationValueFor(BrowserAnimationSequence which) const {
 }
 
 bool SidePanel::ShouldShowAnimation() const {
+  // BrowserOS: animations_disabled_browseros_ used to control animation
   bool should_show_animations =
-      gfx::Animation::ShouldRenderRichAnimation() && !animations_disabled_;
+      gfx::Animation::ShouldRenderRichAnimation() && !animations_disabled_ &&
+      animations_disabled_browseros_;
 #if BUILDFLAG(IS_WIN)
   // Don't show open/close animations for the toolbar height panel on Windows
   // due to jank. The "show from" animation should still run which is the only
