diff --git a/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.h b/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.h
index b4cef54f99df89ac0c7997bbb7187e0ce8845ede..3da16fa2c075bc4c07b7956440feceaeff845918 100644
--- a/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.h
+++ b/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.h
@@ -70,7 +70,7 @@ class SessionRestoreInfoBarDelegate : public ConfirmInfoBarDelegate {
 
   // ConfirmInfoBarDelegate:
   infobars::InfoBarDelegate::InfoBarIdentifier GetIdentifier() const override;
-  const gfx::VectorIcon& GetVectorIcon() const override;
+  int GetIconId() const override;
   bool ShouldExpire(const NavigationDetails& details) const override;
   std::u16string GetMessageText() const override;
   std::u16string GetLinkText() const override;
