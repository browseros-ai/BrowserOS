diff --git a/chrome/browser/ui/views/infobars/infobar_container_view.cc b/chrome/browser/ui/views/infobars/infobar_container_view.cc
index 38cc2a11258cfb55520e3a0afc4b3a600d7d3eca..c9781bc2c65ee1ae3058cd880c5756a10a6c1fc2 100644
--- a/chrome/browser/ui/views/infobars/infobar_container_view.cc
+++ b/chrome/browser/ui/views/infobars/infobar_container_view.cc
@@ -126,8 +126,7 @@ void InfoBarContainerView::Layout(PassKey) {
   // there drawn by the shadow code (so we don't have to extend our bounds out
   // to be able to draw it; see comments in CalculatePreferredSize() on why the
   // shadow is drawn outside the container bounds).
-  content_shadow_->SetBounds(0, top, width(),
-                             content_shadow_->GetPreferredSize().height());
+  content_shadow_->SetBounds(0, top, width(), 1);
 }
 
 gfx::Size InfoBarContainerView::CalculatePreferredSize(
