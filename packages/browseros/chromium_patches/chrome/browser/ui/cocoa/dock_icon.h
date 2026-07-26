diff --git a/chrome/browser/ui/cocoa/dock_icon.h b/chrome/browser/ui/cocoa/dock_icon.h
index 507dcaa601b0f..1f72c0ffc7d54 100644
--- a/chrome/browser/ui/cocoa/dock_icon.h
+++ b/chrome/browser/ui/cocoa/dock_icon.h
@@ -21,6 +21,10 @@
 // Updates the icon. Use the setters below to set the details first.
 - (void)updateIcon;
 
+// Dock variant tint ///////////////////////////////////////////////////////////
+
+- (void)setDockIconVariantColor:(NSColor*)color;
+
 // Download progress ///////////////////////////////////////////////////////////
 
 // Indicates how many downloads are in progress.
