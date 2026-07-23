diff --git a/chrome/browser/ui/startup/default_browser_prompt/default_browser_infobar_delegate.h b/chrome/browser/ui/startup/default_browser_prompt/default_browser_infobar_delegate.h
index 2f2eece414c41285704de005fce2acda224d32b1..207587516423e3a1fae15da362a8bcd65201da62 100644
--- a/chrome/browser/ui/startup/default_browser_prompt/default_browser_infobar_delegate.h
+++ b/chrome/browser/ui/startup/default_browser_prompt/default_browser_infobar_delegate.h
@@ -40,7 +40,7 @@ class DefaultBrowserInfoBarDelegate : public ConfirmInfoBarDelegate {
  private:
   // ConfirmInfoBarDelegate:
   infobars::InfoBarDelegate::InfoBarIdentifier GetIdentifier() const override;
-  const gfx::VectorIcon& GetVectorIcon() const override;
+  int GetIconId() const override;
   bool ShouldExpire(const NavigationDetails& details) const override;
   void InfoBarDismissed() override;
   std::u16string GetMessageText() const override;
