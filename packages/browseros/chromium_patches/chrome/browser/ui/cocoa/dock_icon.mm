diff --git a/chrome/browser/ui/cocoa/dock_icon.mm b/chrome/browser/ui/cocoa/dock_icon.mm
index 3cb3285ab04fa7042369858ae868adc04db50c25..a87d0941f8f567c945e8aa3ed079f50e3c80592d 100644
--- a/chrome/browser/ui/cocoa/dock_icon.mm
+++ b/chrome/browser/ui/cocoa/dock_icon.mm
@@ -30,6 +30,28 @@
     {1, 3, 0.2},
 };
 
+// Tint the current app icon without changing its alpha mask, so product
+// variants retain custom icons and the download badge remains untinted.
+NSImage* AppIconWithVariantTint(NSImage* appIcon, NSSize size, NSColor* tint) {
+  NSImage* tintedIcon = [[NSImage alloc] initWithSize:size];
+  const NSRect iconRect = NSMakeRect(0, 0, size.width, size.height);
+
+  [tintedIcon lockFocus];
+  [appIcon drawInRect:iconRect
+             fromRect:NSZeroRect
+            operation:NSCompositingOperationSourceOver
+             fraction:1.0];
+  [tint setFill];
+  NSRectFillUsingOperation(iconRect, NSCompositingOperationColor);
+  [appIcon drawInRect:iconRect
+             fromRect:NSZeroRect
+            operation:NSCompositingOperationDestinationIn
+             fraction:1.0];
+  [tintedIcon unlockFocus];
+
+  return tintedIcon;
+}
+
 }  // namespace
 
 // A view that draws our dock tile.
@@ -45,6 +67,8 @@ @interface DockTileView : NSView
 // Indicates the amount of progress made of the download. Ranges from [0..1].
 @property(nonatomic) float progress;
 
+@property(nonatomic, strong) NSColor* variantColor;
+
 @end
 
 @implementation DockTileView
@@ -52,6 +76,7 @@ @implementation DockTileView
 @synthesize downloads = _downloads;
 @synthesize indeterminate = _indeterminate;
 @synthesize progress = _progress;
+@synthesize variantColor = _variantColor;
 
 - (void)drawRect:(NSRect)dirtyRect {
   // This needs to draw the current app icon, whether it's using the default
@@ -70,6 +95,9 @@ - (void)drawRect:(NSRect)dirtyRect {
   // Therefore, use [NSImage imageNamed:NSImageNameApplicationIcon].
 
   NSImage* appIcon = [NSImage imageNamed:NSImageNameApplicationIcon];
+  if (_variantColor) {
+    appIcon = AppIconWithVariantTint(appIcon, self.bounds.size, _variantColor);
+  }
   [appIcon drawInRect:self.bounds
              fromRect:NSZeroRect
             operation:NSCompositingOperationSourceOver
@@ -215,6 +243,19 @@ - (void)updateIcon {
   [NSApp.dockTile display];
 }
 
+- (void)setDockIconVariantColor:(NSColor*)color {
+  DCHECK_CURRENTLY_ON(BrowserThread::UI);
+  DockTileView* dockTileView =
+      base::apple::ObjCCast<DockTileView>(NSApp.dockTile.contentView);
+
+  BOOL sameColor = color == [dockTileView variantColor] ||
+                   [color isEqual:[dockTileView variantColor]];
+  if (!sameColor) {
+    [dockTileView setVariantColor:color];
+    _forceUpdate = YES;
+  }
+}
+
 - (void)setDownloads:(int)downloads {
   DCHECK_CURRENTLY_ON(BrowserThread::UI);
   DockTileView* dockTileView =
