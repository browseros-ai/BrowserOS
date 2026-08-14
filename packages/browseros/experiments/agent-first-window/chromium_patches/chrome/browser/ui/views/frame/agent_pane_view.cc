diff --git a/chrome/browser/ui/views/frame/agent_pane_view.cc b/chrome/browser/ui/views/frame/agent_pane_view.cc
new file mode 100644
index 0000000000000000000000000000000000000000..ba72fbce5e7d66658ab17b88bb968d91ad094408
--- /dev/null
+++ b/chrome/browser/ui/views/frame/agent_pane_view.cc
@@ -0,0 +1,274 @@
+// Copyright 2026 The BrowserOS Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/views/frame/agent_pane_view.h"
+
+#include <algorithm>
+#include <memory>
+
+#include "base/check.h"
+#include "base/functional/bind.h"
+#include "base/i18n/rtl.h"
+#include "base/logging.h"
+#include "chrome/app/chrome_command_ids.h"
+#include "chrome/app/vector_icons/vector_icons.h"
+#include "chrome/browser/extensions/browseros_extension_constants.h"
+#include "chrome/browser/extensions/extension_view_host.h"
+#include "chrome/browser/extensions/extension_view_host_factory.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/browser_commands.h"
+#include "chrome/browser/ui/views/extensions/extension_view_views.h"
+#include "chrome/browser/ui/views/frame/browser_view.h"
+#include "content/public/browser/browser_context.h"
+#include "extensions/common/extension.h"
+#include "third_party/skia/include/core/SkColor.h"
+#include "ui/base/metadata/metadata_impl_macros.h"
+#include "ui/gfx/geometry/point.h"
+#include "ui/gfx/geometry/rect.h"
+#include "ui/views/background.h"
+#include "ui/views/controls/button/image_button.h"
+#include "ui/views/controls/button/image_button_factory.h"
+#include "ui/views/controls/label.h"
+#include "ui/views/controls/resize_area.h"
+
+namespace {
+
+constexpr SkColor kAgentPaneBackground = SkColorSetRGB(0xEA, 0xEC, 0xF0);
+constexpr SkColor kAgentTitleBarBackground = SkColorSetRGB(0xDF, 0xE2, 0xE7);
+constexpr char kAgentHomePath[] = "app.html#/home";
+
+}  // namespace
+
+AgentPaneView::AgentPaneView(BrowserView* browser_view)
+    : browser_view_(browser_view) {
+  SetBackground(views::CreateSolidBackground(kAgentPaneBackground));
+
+  if (browser_view_) {
+    registry_observation_.Observe(
+        extensions::ExtensionRegistry::Get(browser_view_->GetProfile()));
+  }
+
+  // Create contents before interactive chrome so the resize area and toggle
+  // stay above the extension WebContents in both paint and event order.
+  EnsureContents();
+
+  drag_strip_ = AddChildView(std::make_unique<views::View>());
+  drag_strip_->SetBackground(
+      views::CreateSolidBackground(kAgentTitleBarBackground));
+
+  auto toggle_button = views::CreateVectorImageButtonWithNativeTheme(
+      base::BindRepeating(&AgentPaneView::ToggleBrowser,
+                          base::Unretained(this)),
+      kOpenInNewIcon, 16);
+  toggle_button->SetTooltipText(u"Show or hide browser");
+  toggle_button->SetPreferredSize(gfx::Size(28, 28));
+  toggle_button->SetFocusBehavior(views::View::FocusBehavior::ALWAYS);
+  toggle_button_ = AddChildView(std::move(toggle_button));
+
+  resize_area_ = AddChildView(std::make_unique<views::ResizeArea>(this));
+  SetShowResizeArea(false);
+}
+
+AgentPaneView::~AgentPaneView() {
+  registry_observation_.Reset();
+  DestroyExtensionContents();
+}
+
+void AgentPaneView::EnsureContents() {
+  if (contents_) {
+    return;
+  }
+
+  if (TryCreateExtensionContents()) {
+    return;
+  }
+
+  ShowPlaceholder();
+}
+
+bool AgentPaneView::TryCreateExtensionContents() {
+  if (!browser_view_ || host_) {
+    return false;
+  }
+
+  const extensions::Extension* extension =
+      extensions::ExtensionRegistry::Get(browser_view_->GetProfile())
+          ->enabled_extensions()
+          .GetByID(extensions::browseros::kAgentV2ExtensionId);
+  if (!extension) {
+    VLOG(1) << "AgentPaneView using placeholder; agent extension is not "
+               "enabled";
+    return false;
+  }
+
+  const GURL url = extension->GetResourceURL(kAgentHomePath);
+  host_ = extensions::ExtensionViewHostFactory::CreateSidePanelHost(
+      url, browser_view_->browser(), /*tab_interface=*/nullptr);
+  if (!host_) {
+    VLOG(1) << "AgentPaneView using placeholder; extension host creation "
+               "failed for "
+            << url;
+    return false;
+  }
+
+  host_->SetCloseHandler(base::BindOnce(
+      &AgentPaneView::HandleCloseExtensionHost, base::Unretained(this)));
+  host_observation_.Observe(host_.get());
+
+  auto extension_view = std::make_unique<ExtensionViewViews>(
+      browser_view_->GetProfile(), host_.get());
+  extension_view->SetVisible(true);
+  contents_ = AddChildViewAt(std::move(extension_view), 0);
+  VLOG(1) << "AgentPaneView created extension host url=" << url;
+  InvalidateLayout();
+  return true;
+}
+
+void AgentPaneView::ShowPlaceholder() {
+  CHECK(!contents_);
+
+  auto placeholder = std::make_unique<views::Label>(
+      u"BrowserOS agent extension not installed");
+  placeholder->SetHorizontalAlignment(gfx::ALIGN_CENTER);
+  contents_ = AddChildViewAt(std::move(placeholder), 0);
+  InvalidateLayout();
+}
+
+void AgentPaneView::DestroyExtensionContents() {
+  if (!host_) {
+    return;
+  }
+
+  host_observation_.Reset();
+  if (contents_) {
+    RemoveChildViewT(contents_);
+    contents_ = nullptr;
+  }
+  host_.reset();
+}
+
+void AgentPaneView::HandleCloseExtensionHost(extensions::ExtensionHost* host) {
+  CHECK_EQ(host, host_.get());
+  DestroyExtensionContents();
+  ShowPlaceholder();
+}
+
+void AgentPaneView::SetShowResizeArea(bool show) {
+  resize_area_->SetVisible(show);
+}
+
+void AgentPaneView::RestoreFocus() {
+  toggle_button_->RequestFocus();
+}
+
+bool AgentPaneView::IsPointInDragStrip(const gfx::Point& point) const {
+  if (!bounds().Contains(point)) {
+    return false;
+  }
+
+  gfx::Point local_point(point.x() - x(), point.y() - y());
+  if (local_point.y() < 0 || local_point.y() >= kTitleBarHeight) {
+    return false;
+  }
+  if (toggle_button_ && toggle_button_->bounds().Contains(local_point)) {
+    return false;
+  }
+  return !resize_area_ || !resize_area_->GetVisible() ||
+         !resize_area_->bounds().Contains(local_point);
+}
+
+void AgentPaneView::Layout(PassKey) {
+  const gfx::Rect bounds = GetLocalBounds();
+  drag_strip_->SetBounds(bounds.x(), bounds.y(), bounds.width(),
+                         kTitleBarHeight);
+
+  const gfx::Size toggle_size = toggle_button_->GetPreferredSize();
+  toggle_button_->SetBounds(
+      std::max(bounds.x(), bounds.right() - toggle_size.width() - 8),
+      bounds.y() + (kTitleBarHeight - toggle_size.height()) / 2,
+      toggle_size.width(), toggle_size.height());
+
+  if (contents_) {
+    contents_->SetBounds(bounds.x(), bounds.y() + kTitleBarHeight,
+                         bounds.width(),
+                         std::max(0, bounds.height() - kTitleBarHeight));
+  }
+  if (resize_area_) {
+    resize_area_->SetBounds(bounds.right() - kResizeAreaWidth, bounds.y(),
+                            kResizeAreaWidth, bounds.height());
+  }
+}
+
+gfx::Size AgentPaneView::GetMinimumSize() const {
+  return gfx::Size(kMinimumWidth, kTitleBarHeight);
+}
+
+void AgentPaneView::OnResize(int resize_amount, bool done_resizing) {
+  if (width_at_resize_start_ < 0) {
+    width_at_resize_start_ = width();
+  }
+
+  // ResizeArea reports logical-direction deltas; the pane is physically fixed
+  // to the left, so undo that mirroring in RTL locales.
+  const int physical_resize_amount =
+      base::i18n::IsRTL() ? -resize_amount : resize_amount;
+  split_width_ =
+      std::max(kMinimumWidth, width_at_resize_start_ + physical_resize_amount);
+  if (done_resizing) {
+    width_at_resize_start_ = -1;
+  }
+  if (browser_view_) {
+    browser_view_->InvalidateLayout();
+  }
+}
+
+void AgentPaneView::ToggleBrowser() {
+  CHECK(browser_view_);
+  chrome::ExecuteCommand(browser_view_->browser(), IDC_TOGGLE_AGENT_SPLIT);
+}
+
+void AgentPaneView::OnExtensionHostDidStopFirstLoad(
+    const extensions::ExtensionHost* host) {
+  if (host == host_.get()) {
+    VLOG(1) << "AgentPaneView extension host loaded url="
+            << host->GetLastCommittedURL();
+  }
+}
+
+void AgentPaneView::OnExtensionLoaded(content::BrowserContext*,
+                                      const extensions::Extension* extension) {
+  if (extension->id() != extensions::browseros::kAgentV2ExtensionId || host_) {
+    return;
+  }
+
+  if (contents_) {
+    RemoveChildViewT(contents_);
+    contents_ = nullptr;
+  }
+  if (!TryCreateExtensionContents()) {
+    ShowPlaceholder();
+  }
+}
+
+void AgentPaneView::OnExtensionUnloaded(content::BrowserContext*,
+                                        const extensions::Extension* extension,
+                                        extensions::UnloadedExtensionReason) {
+  if (extension->id() != extensions::browseros::kAgentV2ExtensionId || !host_) {
+    return;
+  }
+
+  DestroyExtensionContents();
+  ShowPlaceholder();
+}
+
+void AgentPaneView::OnShutdown(extensions::ExtensionRegistry*) {
+  registry_observation_.Reset();
+  DestroyExtensionContents();
+  if (!contents_) {
+    ShowPlaceholder();
+  }
+}
+
+BEGIN_METADATA(AgentPaneView)
+END_METADATA
