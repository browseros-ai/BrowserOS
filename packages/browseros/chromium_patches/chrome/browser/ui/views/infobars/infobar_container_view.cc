diff --git a/chrome/browser/ui/views/infobars/infobar_container_view.cc b/chrome/browser/ui/views/infobars/infobar_container_view.cc
index 82d801f039877..d2f495ccdbac7 100644
--- a/chrome/browser/ui/views/infobars/infobar_container_view.cc
+++ b/chrome/browser/ui/views/infobars/infobar_container_view.cc
@@ -122,8 +122,7 @@ void InfoBarContainerView::Layout(PassKey) {
   // there drawn by the shadow code (so we don't have to extend our bounds out
   // to be able to draw it; see comments in CalculatePreferredSize() on why the
   // shadow is drawn outside the container bounds).
-  content_shadow_->SetBounds(0, top, width(),
-                             content_shadow_->GetPreferredSize().height());
+  content_shadow_->SetBounds(0, top, width(), 1);
 }
 
 gfx::Size InfoBarContainerView::CalculatePreferredSize(
