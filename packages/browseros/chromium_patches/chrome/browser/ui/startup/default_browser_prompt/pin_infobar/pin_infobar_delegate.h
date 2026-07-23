diff --git a/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.h b/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.h
index 3fa4d27bb7d8e310537768d6cc27816360b66a7d..d158d32359f8175917501f911c9eb41f071324a5 100644
--- a/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.h
+++ b/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.h
@@ -39,7 +39,7 @@ class PinInfoBarDelegate : public ConfirmInfoBarDelegate {
 
   // InfoBarDelegate:
   infobars::InfoBarDelegate::InfoBarIdentifier GetIdentifier() const override;
-  const gfx::VectorIcon& GetVectorIcon() const override;
+  int GetIconId() const override;
 
   // ConfirmInfoBarDelegate:
   std::u16string GetMessageText() const override;
