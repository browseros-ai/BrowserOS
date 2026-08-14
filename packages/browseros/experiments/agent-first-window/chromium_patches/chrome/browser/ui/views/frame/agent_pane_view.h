diff --git a/chrome/browser/ui/views/frame/agent_pane_view.h b/chrome/browser/ui/views/frame/agent_pane_view.h
new file mode 100644
index 0000000000000000000000000000000000000000..9b75938d25de489bce30a6dad8010b3d0ef60382
--- /dev/null
+++ b/chrome/browser/ui/views/frame/agent_pane_view.h
@@ -0,0 +1,114 @@
+// Copyright 2026 The BrowserOS Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_VIEWS_FRAME_AGENT_PANE_VIEW_H_
+#define CHROME_BROWSER_UI_VIEWS_FRAME_AGENT_PANE_VIEW_H_
+
+#include <memory>
+
+#include "base/memory/raw_ptr.h"
+#include "base/scoped_observation.h"
+#include "extensions/browser/extension_host_observer.h"
+#include "extensions/browser/extension_registry.h"
+#include "extensions/browser/extension_registry_observer.h"
+#include "ui/base/metadata/metadata_header_macros.h"
+#include "ui/gfx/geometry/size.h"
+#include "ui/views/controls/resize_area_delegate.h"
+#include "ui/views/view.h"
+
+class BrowserView;
+
+namespace gfx {
+class Point;
+}
+
+namespace content {
+class BrowserContext;
+}
+
+namespace extensions {
+class Extension;
+class ExtensionHost;
+class ExtensionViewHost;
+enum class UnloadedExtensionReason;
+}  // namespace extensions
+
+namespace views {
+class ImageButton;
+class ResizeArea;
+}  // namespace views
+
+// The agent-primary surface embedded directly in BrowserView. It owns the
+// title strip, browser toggle, optional extension contents, and split resizer.
+class AgentPaneView : public views::View,
+                      public views::ResizeAreaDelegate,
+                      public extensions::ExtensionHostObserver,
+                      public extensions::ExtensionRegistryObserver {
+  METADATA_HEADER(AgentPaneView, views::View)
+
+ public:
+  static constexpr int kMinimumWidth = 320;
+  static constexpr int kTitleBarHeight = 36;
+  static constexpr int kResizeAreaWidth = 6;
+
+  // `browser_view` may be null only in BrowserViewLayout unit tests.
+  explicit AgentPaneView(BrowserView* browser_view);
+  AgentPaneView(const AgentPaneView&) = delete;
+  AgentPaneView& operator=(const AgentPaneView&) = delete;
+  ~AgentPaneView() override;
+
+  // Creates visible contents on first use. A placeholder is always available
+  // even when the BrowserOS agent extension cannot be loaded.
+  void EnsureContents();
+  void SetShowResizeArea(bool show);
+  void RestoreFocus();
+  int split_width() const { return split_width_; }
+
+  // Returns whether `point`, expressed in BrowserView coordinates, is inside
+  // the non-interactive portion of the pane's title strip.
+  bool IsPointInDragStrip(const gfx::Point& point) const;
+
+  // views::View:
+  void Layout(PassKey) override;
+  gfx::Size GetMinimumSize() const override;
+
+  // views::ResizeAreaDelegate:
+  void OnResize(int resize_amount, bool done_resizing) override;
+
+ private:
+  bool TryCreateExtensionContents();
+  void ShowPlaceholder();
+  void DestroyExtensionContents();
+  void HandleCloseExtensionHost(extensions::ExtensionHost* host);
+  void ToggleBrowser();
+
+  // extensions::ExtensionHostObserver:
+  void OnExtensionHostDidStopFirstLoad(
+      const extensions::ExtensionHost* host) override;
+
+  // extensions::ExtensionRegistryObserver:
+  void OnExtensionLoaded(content::BrowserContext* browser_context,
+                         const extensions::Extension* extension) override;
+  void OnExtensionUnloaded(content::BrowserContext* browser_context,
+                           const extensions::Extension* extension,
+                           extensions::UnloadedExtensionReason reason) override;
+  void OnShutdown(extensions::ExtensionRegistry* registry) override;
+
+  const raw_ptr<BrowserView> browser_view_;
+  raw_ptr<views::View> drag_strip_ = nullptr;
+  raw_ptr<views::ImageButton> toggle_button_ = nullptr;
+  raw_ptr<views::View> contents_ = nullptr;
+  raw_ptr<views::ResizeArea> resize_area_ = nullptr;
+  std::unique_ptr<extensions::ExtensionViewHost> host_;
+  base::ScopedObservation<extensions::ExtensionHost,
+                          extensions::ExtensionHostObserver>
+      host_observation_{this};
+  base::ScopedObservation<extensions::ExtensionRegistry,
+                          extensions::ExtensionRegistryObserver>
+      registry_observation_{this};
+  int split_width_ = 0;
+  int width_at_resize_start_ = -1;
+};
+
+#endif  // CHROME_BROWSER_UI_VIEWS_FRAME_AGENT_PANE_VIEW_H_
