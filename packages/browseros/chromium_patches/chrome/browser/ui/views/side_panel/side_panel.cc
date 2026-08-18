diff --git a/chrome/browser/ui/views/side_panel/side_panel.cc b/chrome/browser/ui/views/side_panel/side_panel.cc
index 90f261de2344dd9674f6b10599b80ffd6a0eb951..1c0d86d2631e4750eed6da85c8058d3363ffebe6 100644
--- a/chrome/browser/ui/views/side_panel/side_panel.cc
+++ b/chrome/browser/ui/views/side_panel/side_panel.cc
@@ -129,7 +129,7 @@ class ContentParentBackground : public views::Background {
     SkPath path = SkPath::RRect(rrect);
     canvas->ClipPath(path, /*do_anti_alias=*/true);
 
-      ThemedBackground::PaintBackground(canvas, view, browser_view_);
+    ThemedBackground::PaintBackground(canvas, view, browser_view_);
   }
 
  private:
@@ -716,8 +716,10 @@ double SidePanel::GetAnimationValueFor(BrowserAnimationSequence which) const {
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
 
