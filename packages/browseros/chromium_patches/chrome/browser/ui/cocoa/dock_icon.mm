diff --git a/chrome/browser/ui/cocoa/dock_icon.mm b/chrome/browser/ui/cocoa/dock_icon.mm
index f08c2b156c0fd..c61eb7974ea6a 100644
--- a/chrome/browser/ui/cocoa/dock_icon.mm
+++ b/chrome/browser/ui/cocoa/dock_icon.mm
@@ -32,6 +32,26 @@
 // The maximum update rate for the dock icon. 200ms = 5fps.
 constexpr int64_t kUpdateFrequencyMs = 200;
 
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
@@ -47,6 +67,8 @@ @interface DockTileView : NSView
 // Indicates the amount of progress made of the download. Ranges from [0..1].
 @property(nonatomic) float progress;
 
+@property(nonatomic, strong) NSColor* variantColor;
+
 @end
 
 @implementation DockTileView
@@ -54,6 +76,7 @@ @implementation DockTileView
 @synthesize downloads = _downloads;
 @synthesize indeterminate = _indeterminate;
 @synthesize progress = _progress;
+@synthesize variantColor = _variantColor;
 
 - (void)drawRect:(NSRect)dirtyRect {
   // This needs to draw the current app icon, whether it's using the default
@@ -72,6 +95,9 @@ - (void)drawRect:(NSRect)dirtyRect {
   // Therefore, use [NSImage imageNamed:NSImageNameApplicationIcon].
 
   NSImage* appIcon = [NSImage imageNamed:NSImageNameApplicationIcon];
+  if (_variantColor) {
+    appIcon = AppIconWithVariantTint(appIcon, self.bounds.size, _variantColor);
+  }
   [appIcon drawInRect:self.bounds
              fromRect:NSZeroRect
             operation:NSCompositingOperationSourceOver
@@ -223,6 +249,19 @@ - (void)updateIcon {
   [dockTile display];
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
