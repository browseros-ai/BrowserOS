diff --git a/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.h b/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.h
index 3135fb5d3fdb622d29b629f3ac7769feea9a3b68..c0ca6d5eb4e891b37317658deeeede4ed93f14b8 100644
--- a/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.h
+++ b/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.h
@@ -30,7 +30,7 @@ class PinInfoBarDelegate : public ConfirmInfoBarDelegate {
 
   // InfoBarDelegate:
   infobars::InfoBarDelegate::InfoBarIdentifier GetIdentifier() const override;
-  const gfx::VectorIcon& GetVectorIcon() const override;
+  int GetIconId() const override;
 
   // ConfirmInfoBarDelegate:
   std::u16string GetMessageText() const override;
