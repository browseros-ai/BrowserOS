diff --git a/chrome/browser/ui/views/side_panel/side_panel.cc b/chrome/browser/ui/views/side_panel/side_panel.cc
index ab8999f84de6992bbf403b92ef5c3a5ad0386390..434d766c2caf635b0bd0acb38533496cb963b048 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.cc
+++ b/chrome/browser/ui/views/side_panel/side_panel.cc
@@ -131,7 +131,7 @@ class ContentParentBackground : public views::Background {
     SkPath path = SkPath::RRect(rrect);
     canvas->ClipPath(path, /*do_anti_alias=*/true);
 
-      ThemedBackground::PaintBackground(canvas, view, browser_view_);
+    ThemedBackground::PaintBackground(canvas, view, browser_view_);
   }
 
  private:
@@ -665,8 +665,10 @@ double SidePanel::GetAnimationValueFor(BrowserAnimationSequence which) const {
 }
 
 bool SidePanel::ShouldShowAnimation() const {
-  bool should_show_animations =
-      gfx::Animation::ShouldRenderRichAnimation() && !animations_disabled_;
+  // BrowserOS: animations_disabled_browseros_ used to control animation
+  bool should_show_animations = gfx::Animation::ShouldRenderRichAnimation() &&
+                                !animations_disabled_ &&
+                                animations_disabled_browseros_;
   return should_show_animations;
 }
 
