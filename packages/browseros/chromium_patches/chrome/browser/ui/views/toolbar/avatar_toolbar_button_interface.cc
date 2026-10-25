diff --git a/chrome/browser/ui/views/toolbar/avatar_toolbar_button_interface.cc b/chrome/browser/ui/views/toolbar/avatar_toolbar_button_interface.cc
index c8f0fd86033d1fe99ed3a1630e302c5e4ca7eb81..28bd32c2e385762b7b52e665aad71daafa0ea2e3 100644
--- a/chrome/browser/ui/views/toolbar/avatar_toolbar_button_interface.cc
+++ b/chrome/browser/ui/views/toolbar/avatar_toolbar_button_interface.cc
@@ -29,6 +29,15 @@ bool AvatarToolbarButtonInterface::CanShowForProfile(Profile* profile) {
 
 views::BubbleAnchor AvatarToolbarButtonInterface::GetBubbleAnchor(
     BrowserWindowInterface& browser) {
+  // BrowserOS: the avatar can be hidden (e.g. by
+  // --disable-browseros-features=AvatarButton), and a hidden view is not a
+  // tracked element. Anchor its bubbles to the app menu button rather than to
+  // nothing; callers such as the incognito clear-data dialog assume an anchor.
+  if (!BrowserElements::From(&browser)->GetElement(
+          kToolbarAvatarButtonElementId)) {
+    return views::BubbleAnchor(BrowserElements::From(&browser)->GetElement(
+        kToolbarAppMenuButtonElementId));
+  }
   return views::BubbleAnchor(BrowserElements::From(&browser)->GetElement(
       kToolbarAvatarButtonElementId));
 }
