diff --git a/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.h b/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.h
index 7fb5a1d2364c2e9b2354fb93a81c442cfa5e7562..dc130791ac7c8912a56d56350f54a04e0549ad70 100644
--- a/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.h
+++ b/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.h
@@ -44,7 +44,7 @@ class SessionRestoreInfoBarDelegate : public ConfirmInfoBarDelegate {
 
   // ConfirmInfoBarDelegate:
   infobars::InfoBarDelegate::InfoBarIdentifier GetIdentifier() const override;
-  const gfx::VectorIcon& GetVectorIcon() const override;
+  int GetIconId() const override;
   bool ShouldExpire(const NavigationDetails& details) const override;
   std::u16string GetMessageText() const override;
   std::u16string GetLinkText() const override;
