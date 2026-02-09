diff --git a/chrome/browser/ui/views/side_panel/side_panel.cc b/chrome/browser/ui/views/side_panel/side_panel.cc
index 56a63f28ca526..773d4ff107c9a 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.cc
+++ b/chrome/browser/ui/views/side_panel/side_panel.cc
@@ -879,8 +879,10 @@ double SidePanel::GetAnimationValueFor(
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
