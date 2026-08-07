diff --git a/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.h b/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.h
index c953edf3ffd56211c01b569bfd55a0bb0ed2937f..431436247ff1ae3b26a3929fd6abf1b42e7a931f 100644
--- a/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.h
+++ b/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.h
@@ -38,7 +38,7 @@ class PdfInfoBarDelegate : public ConfirmInfoBarDelegate {
 
   // InfoBarDelegate:
   infobars::InfoBarDelegate::InfoBarIdentifier GetIdentifier() const override;
-  const gfx::VectorIcon& GetVectorIcon() const override;
+  int GetIconId() const override;
 
   // ConfirmInfoBarDelegate:
   std::u16string GetMessageText() const override;
