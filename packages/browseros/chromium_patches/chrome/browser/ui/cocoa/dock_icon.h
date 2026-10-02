diff --git a/chrome/browser/ui/cocoa/dock_icon.h b/chrome/browser/ui/cocoa/dock_icon.h
index 3d5103dcd291853fe7587676abebf283d7ca1209..61214ea1efea0708a27fdc5b0bd92c9c4fe15a94 100644
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
