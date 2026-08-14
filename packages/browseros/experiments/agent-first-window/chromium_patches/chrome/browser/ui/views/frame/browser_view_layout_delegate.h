diff --git a/chrome/browser/ui/views/frame/browser_view_layout_delegate.h b/chrome/browser/ui/views/frame/browser_view_layout_delegate.h
index 9f393a42d16557582867691b92aa2afbc3bbe60c..3236e3a01d834c7a0c98223836e3bfa1efb9c5fd 100644
--- a/chrome/browser/ui/views/frame/browser_view_layout_delegate.h
+++ b/chrome/browser/ui/views/frame/browser_view_layout_delegate.h
@@ -29,6 +29,8 @@ class BrowserViewLayoutDelegate {
       views::Label& window_title_label) const = 0;
   virtual int GetTopInsetInBrowserView() const = 0;
   virtual bool IsToolbarVisible() const = 0;
+  virtual int GetAgentPaneWidth() const = 0;
+  virtual bool IsAgentPaneOnly() const = 0;
   virtual bool IsBookmarkBarVisible() const = 0;
   virtual bool IsContentsSeparatorEnabled() const = 0;
   virtual ExclusiveAccessBubbleViews* GetExclusiveAccessBubble() const = 0;
