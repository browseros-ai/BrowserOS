diff --git a/chrome/browser/ui/views/infobars/infobar_container_view.cc b/chrome/browser/ui/views/infobars/infobar_container_view.cc
index 20f8ef44504cb298368f50243d2a492a7bc8440d..9ef5eae6566fa2bfaeb3fa6974fde3eb784fdc39 100644
--- a/chrome/browser/ui/views/infobars/infobar_container_view.cc
+++ b/chrome/browser/ui/views/infobars/infobar_container_view.cc
@@ -125,8 +125,7 @@ void InfoBarContainerView::Layout(PassKey) {
   // there drawn by the shadow code (so we don't have to extend our bounds out
   // to be able to draw it; see comments in CalculatePreferredSize() on why the
   // shadow is drawn outside the container bounds).
-  content_shadow_->SetBounds(0, top, width(),
-                             content_shadow_->GetPreferredSize().height());
+  content_shadow_->SetBounds(0, top, width(), 1);
 }
 
 gfx::Size InfoBarContainerView::CalculatePreferredSize(
