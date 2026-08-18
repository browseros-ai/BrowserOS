diff --git a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button.h b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button.h
index d9b07af5c000e4d018613b8d6bd2e9a9bc0d2e17..d9ecfef745994d0d1f2528c58d68dbc84b24bb2b 100644
--- a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button.h
+++ b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button.h
@@ -53,11 +53,13 @@ class PinnedActionToolbarButton : public ToolbarButton {
   }
   void SetActionEngaged(bool action_engaged);
   void UpdateIcon() override;
+  void UpdateLabelVisibility();
   bool ShouldShowEphemerallyInToolbar();
   bool IsIconVisible() { return is_icon_visible_; }
   bool IsPinned() { return pinned_; }
   bool IsPermanent() { return permanent_; }
   views::View* GetImageContainerView() { return image_container_view(); }
+  Browser* GetBrowser() { return browser_; }
 
   bool ShouldSkipExecutionForTesting() { return skip_execution_; }
 
