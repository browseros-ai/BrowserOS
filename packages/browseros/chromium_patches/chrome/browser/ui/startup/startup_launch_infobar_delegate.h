diff --git a/chrome/browser/ui/startup/startup_launch_infobar_delegate.h b/chrome/browser/ui/startup/startup_launch_infobar_delegate.h
index 4c3e07af3b3a4f0aab0c86f311586b69404a83f1..c5627fad5f6676c78c5cd2c12f4a3fea00ea3164 100644
--- a/chrome/browser/ui/startup/startup_launch_infobar_delegate.h
+++ b/chrome/browser/ui/startup/startup_launch_infobar_delegate.h
@@ -37,7 +37,7 @@ class StartupLaunchInfoBarDelegate : public ConfirmInfoBarDelegate {
  private:
   // ConfirmInfoBarDelegate:
   infobars::InfoBarDelegate::InfoBarIdentifier GetIdentifier() const override;
-  const gfx::VectorIcon& GetVectorIcon() const override;
+  int GetIconId() const override;
   bool ShouldExpire(const NavigationDetails& details) const override;
   std::u16string GetMessageText() const override;
   int GetButtons() const override;
