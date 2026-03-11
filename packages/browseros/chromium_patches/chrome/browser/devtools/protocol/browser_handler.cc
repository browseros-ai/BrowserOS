diff --git a/chrome/browser/devtools/protocol/browser_handler.cc b/chrome/browser/devtools/protocol/browser_handler.cc
index 30bd52d09c3fc..2df7c861f26aa 100644
--- a/chrome/browser/devtools/protocol/browser_handler.cc
+++ b/chrome/browser/devtools/protocol/browser_handler.cc
@@ -8,19 +8,32 @@
 #include <vector>
 
 #include "base/functional/bind.h"
+#include "base/memory/raw_ptr.h"
 #include "base/memory/ref_counted_memory.h"
+#include "base/strings/utf_string_conversions.h"
+#include "build/build_config.h"
 #include "chrome/app/chrome_command_ids.h"
 #include "chrome/browser/devtools/chrome_devtools_manager_delegate.h"
 #include "chrome/browser/devtools/devtools_dock_tile.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/profiles/profile_manager.h"
+#include "chrome/browser/ui/browser.h"
 #include "chrome/browser/ui/browser_commands.h"
 #include "chrome/browser/ui/browser_list.h"
+#include "chrome/browser/ui/browser_tabstrip.h"
 #include "chrome/browser/ui/browser_window.h"
+#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h"
 #include "chrome/browser/ui/exclusive_access/exclusive_access_context.h"
+#include "chrome/browser/ui/tabs/tab_enums.h"
 #include "chrome/browser/ui/tabs/tab_strip_model.h"
+#include "chrome/browser/ui/tabs/tab_group_model.h"
+#include "components/tabs/public/tab_group.h"
 #include "components/privacy_sandbox/privacy_sandbox_attestations/privacy_sandbox_attestations.h"
+#include "components/sessions/content/session_tab_helper.h"
+#include "components/tab_groups/tab_group_color.h"
+#include "components/tab_groups/tab_group_id.h"
+#include "components/tab_groups/tab_group_visual_data.h"
 #include "content/public/browser/browser_task_traits.h"
 #include "content/public/browser/browser_thread.h"
 #include "content/public/browser/devtools_agent_host.h"
@@ -30,10 +43,21 @@
 #include "ui/gfx/image/image.h"
 #include "ui/gfx/image/image_png_rep.h"
 
+#if BUILDFLAG(IS_MAC)
+#include "chrome/browser/devtools/protocol/browser_handler_mac.h"
+#endif
+
 using protocol::Response;
 
 namespace {
 
+#if !BUILDFLAG(IS_MAC)
+// Off-screen position used to hide windows while keeping their compositors
+// active. This enables CDP operations like Page.captureScreenshot on hidden
+// windows. Uses cross-platform ui::BaseWindow::SetBounds/ShowInactive APIs.
+constexpr int kOffScreenPosition = -32000;
+#endif
+
 BrowserWindow* GetBrowserWindow(int window_id) {
   BrowserWindow* result = nullptr;
   ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
@@ -72,17 +96,398 @@ std::unique_ptr<protocol::Browser::Bounds> GetBrowserWindowBounds(
       .Build();
 }
 
+BrowserWindowInterface* GetBrowserWindowInterface(int window_id) {
+  BrowserWindowInterface* result = nullptr;
+  ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+      [window_id, &result](BrowserWindowInterface* bwi) {
+        if (bwi->GetSessionID().id() == window_id) {
+          result = bwi;
+          return false;
+        }
+        return true;
+      });
+  return result;
+}
+
+std::string GetBrowserWindowType(BrowserWindowInterface::Type type) {
+  switch (type) {
+    case BrowserWindowInterface::TYPE_NORMAL:
+      return "normal";
+    case BrowserWindowInterface::TYPE_POPUP:
+      return "popup";
+    case BrowserWindowInterface::TYPE_APP:
+      return "app";
+#if !BUILDFLAG(IS_ANDROID)
+    case BrowserWindowInterface::TYPE_DEVTOOLS:
+      return "devtools";
+#endif
+    case BrowserWindowInterface::TYPE_APP_POPUP:
+      return "app_popup";
+#if BUILDFLAG(IS_CHROMEOS)
+    case BrowserWindowInterface::TYPE_CUSTOM_TAB:
+      return "normal";
+#endif
+#if !BUILDFLAG(IS_ANDROID)
+    case BrowserWindowInterface::TYPE_PICTURE_IN_PICTURE:
+      return "picture_in_picture";
+#endif
+  }
+  return "normal";
+}
+
+BrowserWindowInterface::Type ParseWindowType(const std::string& type_str) {
+  if (type_str == "popup")
+    return BrowserWindowInterface::TYPE_POPUP;
+  if (type_str == "app")
+    return BrowserWindowInterface::TYPE_APP;
+#if !BUILDFLAG(IS_ANDROID)
+  if (type_str == "devtools")
+    return BrowserWindowInterface::TYPE_DEVTOOLS;
+#endif
+  if (type_str == "app_popup")
+    return BrowserWindowInterface::TYPE_APP_POPUP;
+#if !BUILDFLAG(IS_ANDROID)
+  if (type_str == "picture_in_picture")
+    return BrowserWindowInterface::TYPE_PICTURE_IN_PICTURE;
+#endif
+  return BrowserWindowInterface::TYPE_NORMAL;
+}
+
+std::unique_ptr<protocol::Browser::WindowInfo> BuildWindowInfo(
+    BrowserWindowInterface* bwi,
+    bool is_hidden = false) {
+  ui::BaseWindow* window = bwi->GetWindow();
+  TabStripModel* tab_strip = bwi->GetTabStripModel();
+
+  auto info = protocol::Browser::WindowInfo::Create()
+                  .SetWindowId(bwi->GetSessionID().id())
+                  .SetWindowType(GetBrowserWindowType(bwi->GetType()))
+                  .SetBounds(GetBrowserWindowBounds(window))
+                  .SetIsActive(bwi->IsActive())
+                  .SetIsVisible(!is_hidden && window->IsVisible())
+                  .SetTabCount(tab_strip->count())
+                  .Build();
+
+  content::WebContents* active_wc = tab_strip->GetActiveWebContents();
+  if (active_wc) {
+    SessionID tab_id = sessions::SessionTabHelper::IdForTab(active_wc);
+    if (tab_id.is_valid()) {
+      info->SetActiveTabId(tab_id.id());
+    }
+  }
+
+  Profile* profile = bwi->GetProfile();
+  if (profile) {
+    info->SetBrowserContextId(profile->GetDebugName());
+  }
+
+  return info;
+}
+
+std::string SerializeGroupId(const tab_groups::TabGroupId& id) {
+  return id.token().ToString();
+}
+
+std::unique_ptr<protocol::Browser::TabInfo> BuildTabInfo(
+    content::WebContents* wc,
+    BrowserWindowInterface* bwi,
+    int tab_index,
+    bool is_hidden) {
+  SessionID sid = sessions::SessionTabHelper::IdForTab(wc);
+  scoped_refptr<content::DevToolsAgentHost> host =
+      content::DevToolsAgentHost::GetOrCreateFor(wc);
+
+  bool is_active = false;
+  bool is_pinned = false;
+  if (!is_hidden && bwi) {
+    TabStripModel* tab_strip = bwi->GetTabStripModel();
+    is_active = tab_strip->GetActiveWebContents() == wc;
+    is_pinned = tab_strip->IsTabPinned(tab_index);
+  }
+
+  auto info = protocol::Browser::TabInfo::Create()
+                  .SetTabId(sid.id())
+                  .SetTargetId(host->GetId())
+                  .SetUrl(wc->GetVisibleURL().spec())
+                  .SetTitle(base::UTF16ToUTF8(wc->GetTitle()))
+                  .SetIsActive(is_active)
+                  .SetIsLoading(wc->IsLoading())
+                  .SetLoadProgress(wc->GetLoadProgress())
+                  .SetIsPinned(is_pinned)
+                  .SetIsHidden(is_hidden)
+                  .Build();
+
+  if (!is_hidden && bwi) {
+    info->SetWindowId(bwi->GetSessionID().id());
+    info->SetIndex(tab_index);
+    std::optional<tab_groups::TabGroupId> group =
+        bwi->GetTabStripModel()->GetTabGroupForTab(tab_index);
+    if (group.has_value()) {
+      info->SetGroupId(SerializeGroupId(group.value()));
+    }
+  }
+
+  Profile* profile =
+      Profile::FromBrowserContext(wc->GetBrowserContext());
+  if (profile) {
+    info->SetBrowserContextId(profile->GetDebugName());
+  }
+
+  return info;
+}
+
+struct TabLookupResult {
+  raw_ptr<content::WebContents> web_contents = nullptr;
+  raw_ptr<BrowserWindowInterface> bwi = nullptr;
+  int tab_index = -1;
+  bool is_hidden = false;
+};
+
+Response ResolveTabIdentifier(std::optional<std::string> target_id,
+                              std::optional<int> tab_id,
+                              const base::flat_set<int>& hidden_window_ids,
+                              TabLookupResult* result) {
+  if (target_id.has_value() && tab_id.has_value()) {
+    return Response::InvalidParams(
+        "Specify either targetId or tabId, not both");
+  }
+  if (!target_id.has_value() && !tab_id.has_value()) {
+    return Response::InvalidParams(
+        "Either targetId or tabId must be specified");
+  }
+
+  if (target_id.has_value()) {
+    auto host = content::DevToolsAgentHost::GetForId(target_id.value());
+    if (!host)
+      return Response::ServerError("No target with given id");
+    content::WebContents* wc = host->GetWebContents();
+    if (!wc)
+      return Response::ServerError("No web contents in the target");
+
+    BrowserWindowInterface* found_bwi = nullptr;
+    int found_index = -1;
+    ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+        [wc, &found_bwi, &found_index](BrowserWindowInterface* bwi) {
+          TabStripModel* tab_strip = bwi->GetTabStripModel();
+          int idx = tab_strip->GetIndexOfWebContents(wc);
+          if (idx != TabStripModel::kNoTab) {
+            found_bwi = bwi;
+            found_index = idx;
+            return false;
+          }
+          return true;
+        });
+
+    if (!found_bwi)
+      return Response::ServerError("No tab with given id");
+
+    result->web_contents = wc;
+    result->bwi = found_bwi;
+    result->tab_index = found_index;
+    result->is_hidden =
+        hidden_window_ids.contains(found_bwi->GetSessionID().id());
+    return Response::Success();
+  }
+
+  // tab_id provided
+  int tid = tab_id.value();
+
+  BrowserWindowInterface* found_bwi = nullptr;
+  content::WebContents* found_wc = nullptr;
+  int found_index = -1;
+  ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+      [tid, &found_bwi, &found_wc,
+       &found_index](BrowserWindowInterface* bwi) {
+        TabStripModel* tab_strip = bwi->GetTabStripModel();
+        for (int i = 0; i < tab_strip->count(); ++i) {
+          content::WebContents* wc = tab_strip->GetWebContentsAt(i);
+          SessionID sid = sessions::SessionTabHelper::IdForTab(wc);
+          if (sid.is_valid() && sid.id() == tid) {
+            found_bwi = bwi;
+            found_wc = wc;
+            found_index = i;
+            return false;
+          }
+        }
+        return true;
+      });
+
+  if (!found_wc)
+    return Response::ServerError("No tab with given id");
+
+  result->web_contents = found_wc;
+  result->bwi = found_bwi;
+  result->tab_index = found_index;
+  result->is_hidden =
+      hidden_window_ids.contains(found_bwi->GetSessionID().id());
+  return Response::Success();
+}
+
+std::optional<tab_groups::TabGroupId> DeserializeGroupId(
+    const std::string& s) {
+  auto token = base::Token::FromString(s);
+  if (!token) {
+    return std::nullopt;
+  }
+  return tab_groups::TabGroupId::FromRawToken(*token);
+}
+
+std::string TabGroupColorToString(tab_groups::TabGroupColorId color) {
+  switch (color) {
+    case tab_groups::TabGroupColorId::kGrey:
+      return "grey";
+    case tab_groups::TabGroupColorId::kBlue:
+      return "blue";
+    case tab_groups::TabGroupColorId::kRed:
+      return "red";
+    case tab_groups::TabGroupColorId::kYellow:
+      return "yellow";
+    case tab_groups::TabGroupColorId::kGreen:
+      return "green";
+    case tab_groups::TabGroupColorId::kPink:
+      return "pink";
+    case tab_groups::TabGroupColorId::kPurple:
+      return "purple";
+    case tab_groups::TabGroupColorId::kCyan:
+      return "cyan";
+    case tab_groups::TabGroupColorId::kOrange:
+      return "orange";
+    default:
+      return "grey";
+  }
+}
+
+std::optional<tab_groups::TabGroupColorId> ParseTabGroupColor(
+    const std::string& s) {
+  if (s == "grey") return tab_groups::TabGroupColorId::kGrey;
+  if (s == "blue") return tab_groups::TabGroupColorId::kBlue;
+  if (s == "red") return tab_groups::TabGroupColorId::kRed;
+  if (s == "yellow") return tab_groups::TabGroupColorId::kYellow;
+  if (s == "green") return tab_groups::TabGroupColorId::kGreen;
+  if (s == "pink") return tab_groups::TabGroupColorId::kPink;
+  if (s == "purple") return tab_groups::TabGroupColorId::kPurple;
+  if (s == "cyan") return tab_groups::TabGroupColorId::kCyan;
+  if (s == "orange") return tab_groups::TabGroupColorId::kOrange;
+  return std::nullopt;
+}
+
+std::unique_ptr<protocol::Browser::TabGroupInfo> BuildTabGroupInfo(
+    BrowserWindowInterface* bwi,
+    const tab_groups::TabGroupId& group_id) {
+  TabStripModel* strip = bwi->GetTabStripModel();
+  const tab_groups::TabGroupVisualData* visual =
+      strip->group_model()->GetTabGroup(group_id)->visual_data();
+
+  auto tab_ids = std::make_unique<protocol::Array<int>>();
+  for (int i = 0; i < strip->count(); ++i) {
+    if (strip->GetTabGroupForTab(i) == group_id) {
+      int tid = sessions::SessionTabHelper::IdForTab(
+                    strip->GetWebContentsAt(i))
+                    .id();
+      tab_ids->push_back(tid);
+    }
+  }
+
+  return protocol::Browser::TabGroupInfo::Create()
+      .SetGroupId(SerializeGroupId(group_id))
+      .SetWindowId(bwi->GetSessionID().id())
+      .SetTitle(base::UTF16ToUTF8(visual->title()))
+      .SetColor(TabGroupColorToString(visual->color()))
+      .SetCollapsed(visual->is_collapsed())
+      .SetTabIds(std::move(tab_ids))
+      .Build();
+}
+
+struct GroupLookupResult {
+  raw_ptr<BrowserWindowInterface> bwi = nullptr;
+  tab_groups::TabGroupId group_id = tab_groups::TabGroupId::CreateEmpty();
+};
+
+Response ResolveGroupId(const std::string& group_id_str,
+                        GroupLookupResult* result) {
+  auto gid = DeserializeGroupId(group_id_str);
+  if (!gid) {
+    return Response::InvalidParams("Invalid group ID format");
+  }
+
+  BrowserWindowInterface* found = nullptr;
+  ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+      [&gid, &found](BrowserWindowInterface* bwi) {
+        auto* gm = bwi->GetTabStripModel()->group_model();
+        if (gm && gm->ContainsTabGroup(*gid)) {
+          found = bwi;
+          return false;
+        }
+        return true;
+      });
+
+  if (!found) {
+    return Response::ServerError("Tab group not found");
+  }
+
+  result->bwi = found;
+  result->group_id = *gid;
+  return Response::Success();
+}
+
+Response ResolveTabIdsToIndices(const protocol::Array<int>& tab_ids,
+                                BrowserWindowInterface** out_bwi,
+                                std::vector<int>* out_indices) {
+  if (tab_ids.empty()) {
+    return Response::InvalidParams("tabIds must not be empty");
+  }
+
+  *out_bwi = nullptr;
+  out_indices->clear();
+
+  for (int tid : tab_ids) {
+    BrowserWindowInterface* found_bwi = nullptr;
+    int found_index = -1;
+
+    ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+        [tid, &found_bwi, &found_index](BrowserWindowInterface* bwi) {
+          TabStripModel* tab_strip = bwi->GetTabStripModel();
+          for (int i = 0; i < tab_strip->count(); ++i) {
+            content::WebContents* wc = tab_strip->GetWebContentsAt(i);
+            SessionID sid = sessions::SessionTabHelper::IdForTab(wc);
+            if (sid.is_valid() && sid.id() == tid) {
+              found_bwi = bwi;
+              found_index = i;
+              return false;
+            }
+          }
+          return true;
+        });
+
+    if (!found_bwi) {
+      return Response::ServerError("No tab with given id");
+    }
+
+    if (*out_bwi && *out_bwi != found_bwi) {
+      return Response::InvalidParams(
+          "All tabs must be in the same window");
+    }
+
+    *out_bwi = found_bwi;
+    out_indices->push_back(found_index);
+  }
+
+  return Response::Success();
+}
+
 }  // namespace
 
 BrowserHandler::BrowserHandler(protocol::UberDispatcher* dispatcher,
                                const std::string& target_id)
     : target_id_(target_id) {
-  // Dispatcher can be null in tests.
   if (dispatcher)
     protocol::Browser::Dispatcher::wire(dispatcher, this);
 }
 
-BrowserHandler::~BrowserHandler() = default;
+BrowserHandler::~BrowserHandler() {
+  hidden_window_per_profile_.clear();
+  hidden_window_ids_.clear();
+}
 
 Response BrowserHandler::GetWindowForTarget(
     std::optional<std::string> target_id,
@@ -120,6 +525,65 @@ Response BrowserHandler::GetWindowForTarget(
   return Response::Success();
 }
 
+Response BrowserHandler::GetTabForTarget(
+    std::optional<std::string> target_id,
+    int* out_tab_id,
+    int* out_window_id) {
+  auto host =
+      content::DevToolsAgentHost::GetForId(target_id.value_or(target_id_));
+  if (!host)
+    return Response::ServerError("No target with given id");
+  content::WebContents* web_contents = host->GetWebContents();
+  if (!web_contents)
+    return Response::ServerError("No web contents in the target");
+
+  SessionID tab_id = sessions::SessionTabHelper::IdForTab(web_contents);
+  if (!tab_id.is_valid())
+    return Response::ServerError("No tab id for target");
+
+  *out_tab_id = tab_id.id();
+
+  SessionID window_id =
+      sessions::SessionTabHelper::IdForWindowContainingTab(web_contents);
+  *out_window_id = window_id.is_valid() ? window_id.id() : -1;
+  return Response::Success();
+}
+
+Response BrowserHandler::GetTargetForTab(
+    int tab_id,
+    std::string* out_target_id,
+    int* out_window_id) {
+  content::WebContents* found_contents = nullptr;
+  int found_window_id = -1;
+  ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+      [tab_id, &found_contents,
+       &found_window_id](BrowserWindowInterface* browser_window_interface) {
+        TabStripModel* tab_strip = browser_window_interface->GetTabStripModel();
+        for (int i = 0; i < tab_strip->count(); ++i) {
+          content::WebContents* wc = tab_strip->GetWebContentsAt(i);
+          SessionID sid = sessions::SessionTabHelper::IdForTab(wc);
+          if (sid.is_valid() && sid.id() == tab_id) {
+            found_contents = wc;
+            found_window_id =
+                browser_window_interface->GetSessionID().id();
+            return false;
+          }
+        }
+        return true;
+      });
+  if (!found_contents)
+    return Response::ServerError("No tab with given id");
+
+  scoped_refptr<content::DevToolsAgentHost> host =
+      content::DevToolsAgentHost::GetOrCreateFor(found_contents);
+  if (!host)
+    return Response::ServerError("No target for tab");
+
+  *out_target_id = host->GetId();
+  *out_window_id = found_window_id;
+  return Response::Success();
+}
+
 Response BrowserHandler::GetWindowBounds(
     int window_id,
     std::unique_ptr<protocol::Browser::Bounds>* out_bounds) {
@@ -297,3 +761,901 @@ protocol::Response BrowserHandler::AddPrivacySandboxEnrollmentOverride(
       net::SchemefulSite(url_to_add));
   return Response::Success();
 }
+
+// --- Window Management ---
+
+Response BrowserHandler::GetWindows(
+    std::unique_ptr<protocol::Array<protocol::Browser::WindowInfo>>*
+        out_windows) {
+  auto windows =
+      std::make_unique<protocol::Array<protocol::Browser::WindowInfo>>();
+  ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+      [&](BrowserWindowInterface* bwi) {
+        bool is_hidden =
+            hidden_window_ids_.contains(bwi->GetSessionID().id());
+        windows->push_back(BuildWindowInfo(bwi, is_hidden));
+        return true;
+      });
+  *out_windows = std::move(windows);
+  return Response::Success();
+}
+
+Response BrowserHandler::GetActiveWindow(
+    std::unique_ptr<protocol::Browser::WindowInfo>* out_window) {
+  BrowserWindowInterface* bwi =
+      GetLastActiveBrowserWindowInterfaceWithAnyProfile();
+  if (bwi) {
+    bool is_hidden = hidden_window_ids_.contains(bwi->GetSessionID().id());
+    *out_window = BuildWindowInfo(bwi, is_hidden);
+  }
+  return Response::Success();
+}
+
+Response BrowserHandler::CreateWindow(
+    std::optional<std::string> url,
+    std::unique_ptr<protocol::Browser::Bounds> bounds,
+    std::optional<std::string> window_type,
+    std::optional<bool> hidden,
+    std::optional<std::string> browser_context_id,
+    std::unique_ptr<protocol::Browser::WindowInfo>* out_window) {
+  Profile* profile = nullptr;
+  BrowserWindowInterface* last_active =
+      GetLastActiveBrowserWindowInterfaceWithAnyProfile();
+  if (last_active) {
+    profile = last_active->GetProfile();
+  }
+  if (!profile) {
+    return Response::ServerError("No profile available");
+  }
+
+  BrowserWindowInterface::Type type = BrowserWindowInterface::TYPE_NORMAL;
+  if (window_type.has_value()) {
+    type = ParseWindowType(window_type.value());
+  }
+
+  Browser::CreateParams params(type, profile, true);
+  if (bounds) {
+    params.initial_bounds =
+        gfx::Rect(bounds->GetLeft(0), bounds->GetTop(0),
+                   bounds->GetWidth(0), bounds->GetHeight(0));
+  }
+
+  Browser* browser = Browser::Create(params);
+
+  GURL navigate_url = url.has_value() ? GURL(url.value()) : GURL();
+  chrome::AddTabAt(browser, navigate_url, -1, true);
+
+  if (hidden.value_or(false)) {
+    MakeWindowHidden(browser);
+  } else {
+    browser->window()->Show();
+  }
+
+  BrowserWindowInterface* bwi = GetBrowserWindowInterface(
+      browser->session_id().id());
+  if (!bwi) {
+    return Response::ServerError("Failed to create window");
+  }
+
+  *out_window = BuildWindowInfo(bwi, hidden.value_or(false));
+  return Response::Success();
+}
+
+Response BrowserHandler::CloseWindow(int window_id) {
+  BrowserWindowInterface* bwi = GetBrowserWindowInterface(window_id);
+  if (!bwi) {
+    return Response::ServerError("Browser window not found");
+  }
+  hidden_window_ids_.erase(window_id);
+  // Clean up hidden_window_per_profile_ if this was a hidden window.
+  Browser* browser = bwi->GetBrowserForMigrationOnly();
+  for (auto it = hidden_window_per_profile_.begin();
+       it != hidden_window_per_profile_.end(); ++it) {
+    if (it->second == browser) {
+      hidden_window_per_profile_.erase(it);
+      break;
+    }
+  }
+  bwi->GetTabStripModel()->CloseAllTabs();
+  bwi->GetWindow()->Close();
+  return Response::Success();
+}
+
+Response BrowserHandler::ActivateWindow(int window_id) {
+  BrowserWindowInterface* bwi = GetBrowserWindowInterface(window_id);
+  if (!bwi) {
+    return Response::ServerError("Browser window not found");
+  }
+  bwi->GetWindow()->Activate();
+  return Response::Success();
+}
+
+Response BrowserHandler::ShowWindow(int window_id) {
+  BrowserWindowInterface* bwi = GetBrowserWindowInterface(window_id);
+  if (!bwi) {
+    return Response::ServerError("Browser window not found");
+  }
+  if (hidden_window_ids_.contains(window_id)) {
+    MakeWindowVisible(bwi);
+  }
+  bwi->GetWindow()->Show();
+  return Response::Success();
+}
+
+Response BrowserHandler::HideWindow(int window_id) {
+  BrowserWindowInterface* bwi = GetBrowserWindowInterface(window_id);
+  if (!bwi) {
+    return Response::ServerError("Browser window not found");
+  }
+  Browser* browser = bwi->GetBrowserForMigrationOnly();
+  MakeWindowHidden(browser);
+  return Response::Success();
+}
+
+// --- Tab Management ---
+
+Response BrowserHandler::GetTabs(
+    std::optional<int> window_id,
+    std::optional<bool> include_hidden,
+    std::unique_ptr<protocol::Array<protocol::Browser::TabInfo>>* out_tabs) {
+  auto tabs =
+      std::make_unique<protocol::Array<protocol::Browser::TabInfo>>();
+
+  if (window_id.has_value()) {
+    BrowserWindowInterface* bwi =
+        GetBrowserWindowInterface(window_id.value());
+    if (!bwi) {
+      return Response::ServerError("Browser window not found");
+    }
+    bool is_hidden = hidden_window_ids_.contains(bwi->GetSessionID().id());
+    TabStripModel* tab_strip = bwi->GetTabStripModel();
+    for (int i = 0; i < tab_strip->count(); ++i) {
+      tabs->push_back(
+          BuildTabInfo(tab_strip->GetWebContentsAt(i), bwi, i, is_hidden));
+    }
+  } else {
+    ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+        [&tabs, this](BrowserWindowInterface* bwi) {
+          bool is_hidden =
+              hidden_window_ids_.contains(bwi->GetSessionID().id());
+          if (is_hidden) {
+            return true;
+          }
+          TabStripModel* tab_strip = bwi->GetTabStripModel();
+          for (int i = 0; i < tab_strip->count(); ++i) {
+            tabs->push_back(
+                BuildTabInfo(tab_strip->GetWebContentsAt(i), bwi, i, false));
+          }
+          return true;
+        });
+  }
+
+  if (include_hidden.value_or(false)) {
+    ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+        [&tabs, this](BrowserWindowInterface* bwi) {
+          if (!hidden_window_ids_.contains(bwi->GetSessionID().id())) {
+            return true;
+          }
+          TabStripModel* tab_strip = bwi->GetTabStripModel();
+          for (int i = 0; i < tab_strip->count(); ++i) {
+            tabs->push_back(
+                BuildTabInfo(tab_strip->GetWebContentsAt(i), bwi, i, true));
+          }
+          return true;
+        });
+  }
+
+  *out_tabs = std::move(tabs);
+  return Response::Success();
+}
+
+Response BrowserHandler::GetActiveTab(
+    std::optional<int> window_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  BrowserWindowInterface* bwi = nullptr;
+  if (window_id.has_value()) {
+    bwi = GetBrowserWindowInterface(window_id.value());
+    if (!bwi) {
+      return Response::ServerError("Browser window not found");
+    }
+  } else {
+    bwi = GetLastActiveBrowserWindowInterfaceWithAnyProfile();
+  }
+
+  if (bwi) {
+    TabStripModel* tab_strip = bwi->GetTabStripModel();
+    content::WebContents* active_wc = tab_strip->GetActiveWebContents();
+    if (active_wc) {
+      int index = tab_strip->GetIndexOfWebContents(active_wc);
+      *out_tab = BuildTabInfo(active_wc, bwi, index, false);
+    }
+  }
+  return Response::Success();
+}
+
+Response BrowserHandler::GetTabInfo(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  *out_tab = BuildTabInfo(lookup.web_contents, lookup.bwi, lookup.tab_index,
+                          lookup.is_hidden);
+  return Response::Success();
+}
+
+Response BrowserHandler::CreateTab(
+    std::optional<std::string> url,
+    std::optional<int> window_id,
+    std::optional<int> index,
+    std::optional<bool> background,
+    std::optional<bool> pinned,
+    std::optional<bool> hidden,
+    std::optional<std::string> browser_context_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  bool is_hidden = hidden.value_or(false);
+
+  if (is_hidden) {
+    if (pinned.value_or(false)) {
+      return Response::InvalidParams("Cannot pin a hidden tab");
+    }
+
+    Profile* profile = nullptr;
+    BrowserWindowInterface* last_active =
+        GetLastActiveBrowserWindowInterfaceWithAnyProfile();
+    if (last_active) {
+      profile = last_active->GetProfile();
+    }
+    if (!profile) {
+      return Response::ServerError("No profile available");
+    }
+
+    Browser* hidden_browser = GetOrCreateHiddenWindow(profile);
+    if (!hidden_browser) {
+      return Response::ServerError("Failed to create hidden window for tab");
+    }
+
+    GURL navigate_url = url.has_value() ? GURL(url.value()) : GURL();
+    chrome::AddTabAt(hidden_browser, navigate_url, -1, false);
+
+    TabStripModel* tab_strip = hidden_browser->tab_strip_model();
+    int new_index = tab_strip->count() - 1;
+    content::WebContents* wc = tab_strip->GetWebContentsAt(new_index);
+    if (!wc) {
+      return Response::ServerError("Failed to create hidden tab");
+    }
+
+    BrowserWindowInterface* bwi = GetBrowserWindowInterface(
+        hidden_browser->session_id().id());
+    *out_tab = BuildTabInfo(wc, bwi, new_index, true);
+    return Response::Success();
+  }
+
+  // Normal (visible) tab creation.
+  BrowserWindowInterface* bwi = nullptr;
+  if (window_id.has_value()) {
+    bwi = GetBrowserWindowInterface(window_id.value());
+    if (!bwi) {
+      return Response::ServerError("Browser window not found");
+    }
+  } else {
+    bwi = GetLastActiveBrowserWindowInterfaceWithAnyProfile();
+  }
+  if (!bwi) {
+    return Response::ServerError("No browser window available");
+  }
+
+  Browser* browser = bwi->GetBrowserForMigrationOnly();
+  GURL navigate_url = url.has_value() ? GURL(url.value()) : GURL();
+  int insert_index = index.value_or(-1);
+  bool foreground = !background.value_or(false);
+
+  content::WebContents* new_wc = chrome::AddAndReturnTabAt(
+      browser, navigate_url, insert_index, foreground);
+  if (!new_wc) {
+    return Response::ServerError("Failed to create tab");
+  }
+
+  TabStripModel* tab_strip = bwi->GetTabStripModel();
+  int new_index = tab_strip->GetIndexOfWebContents(new_wc);
+
+  if (pinned.value_or(false) && new_index != TabStripModel::kNoTab) {
+    new_index = tab_strip->SetTabPinned(new_index, true);
+  }
+
+  *out_tab = BuildTabInfo(new_wc, bwi, new_index, false);
+  return Response::Success();
+}
+
+Response BrowserHandler::CloseTab(std::optional<std::string> target_id,
+                                  std::optional<int> tab_id) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  TabStripModel* tab_strip = lookup.bwi->GetTabStripModel();
+  tab_strip->CloseWebContentsAt(lookup.tab_index,
+                                TabCloseTypes::CLOSE_CREATE_HISTORICAL_TAB);
+  return Response::Success();
+}
+
+Response BrowserHandler::ActivateTab(std::optional<std::string> target_id,
+                                     std::optional<int> tab_id) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  if (lookup.is_hidden) {
+    return Response::InvalidParams(
+        "Cannot activate a hidden tab. Use showTab first.");
+  }
+
+  lookup.bwi->GetTabStripModel()->ActivateTabAt(lookup.tab_index);
+  lookup.bwi->GetWindow()->Activate();
+  return Response::Success();
+}
+
+Response BrowserHandler::MoveTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::optional<int> window_id,
+    std::optional<int> index,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  if (lookup.is_hidden) {
+    return Response::InvalidParams(
+        "Cannot move a hidden tab. Use showTab first.");
+  }
+
+  BrowserWindowInterface* target_bwi = lookup.bwi;
+
+  if (window_id.has_value()) {
+    BrowserWindowInterface* new_bwi =
+        GetBrowserWindowInterface(window_id.value());
+    if (!new_bwi) {
+      return Response::ServerError("Browser window not found");
+    }
+
+    if (new_bwi != lookup.bwi) {
+      // Cross-window move.
+      TabStripModel* source_strip = lookup.bwi->GetTabStripModel();
+      std::unique_ptr<content::WebContents> detached_wc =
+          source_strip->DetachWebContentsAtForInsertion(lookup.tab_index);
+
+      TabStripModel* target_strip = new_bwi->GetTabStripModel();
+      int insert_index =
+          index.has_value() ? index.value() : target_strip->count();
+      target_strip->InsertWebContentsAt(insert_index, std::move(detached_wc),
+                                        AddTabTypes::ADD_NONE);
+
+      int final_index =
+          target_strip->GetIndexOfWebContents(lookup.web_contents);
+      *out_tab =
+          BuildTabInfo(lookup.web_contents, new_bwi, final_index, false);
+      return Response::Success();
+    }
+    target_bwi = new_bwi;
+  }
+
+  // Same-window move or no window specified.
+  if (index.has_value()) {
+    TabStripModel* tab_strip = target_bwi->GetTabStripModel();
+    int new_index =
+        tab_strip->MoveWebContentsAt(lookup.tab_index, index.value(), false);
+    *out_tab =
+        BuildTabInfo(lookup.web_contents, target_bwi, new_index, false);
+  } else {
+    *out_tab = BuildTabInfo(lookup.web_contents, target_bwi,
+                            lookup.tab_index, false);
+  }
+  return Response::Success();
+}
+
+Response BrowserHandler::DuplicateTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  if (lookup.is_hidden) {
+    return Response::InvalidParams("Cannot duplicate a hidden tab");
+  }
+
+  Browser* browser = lookup.bwi->GetBrowserForMigrationOnly();
+  content::WebContents* new_wc =
+      chrome::DuplicateTabAt(browser, lookup.tab_index);
+  if (!new_wc) {
+    return Response::ServerError("Failed to duplicate tab");
+  }
+
+  TabStripModel* tab_strip = lookup.bwi->GetTabStripModel();
+  int new_index = tab_strip->GetIndexOfWebContents(new_wc);
+  *out_tab = BuildTabInfo(new_wc, lookup.bwi, new_index, false);
+  return Response::Success();
+}
+
+Response BrowserHandler::PinTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  if (lookup.is_hidden) {
+    return Response::InvalidParams("Cannot pin a hidden tab");
+  }
+
+  TabStripModel* tab_strip = lookup.bwi->GetTabStripModel();
+  int new_index = tab_strip->SetTabPinned(lookup.tab_index, true);
+  *out_tab =
+      BuildTabInfo(lookup.web_contents, lookup.bwi, new_index, false);
+  return Response::Success();
+}
+
+Response BrowserHandler::UnpinTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  if (lookup.is_hidden) {
+    return Response::InvalidParams("Cannot unpin a hidden tab");
+  }
+
+  TabStripModel* tab_strip = lookup.bwi->GetTabStripModel();
+  int new_index = tab_strip->SetTabPinned(lookup.tab_index, false);
+  *out_tab =
+      BuildTabInfo(lookup.web_contents, lookup.bwi, new_index, false);
+  return Response::Success();
+}
+
+Response BrowserHandler::ShowTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::optional<int> window_id,
+    std::optional<int> index,
+    std::optional<bool> activate,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  if (!lookup.is_hidden) {
+    return Response::InvalidParams("Tab is not hidden");
+  }
+
+  // Detach from the hidden window.
+  TabStripModel* source_strip = lookup.bwi->GetTabStripModel();
+  std::unique_ptr<content::WebContents> detached =
+      source_strip->DetachWebContentsAtForInsertion(lookup.tab_index);
+  if (!detached) {
+    return Response::ServerError("Failed to detach hidden tab");
+  }
+
+  // Find target visible window.
+  BrowserWindowInterface* target_bwi = nullptr;
+  if (window_id.has_value()) {
+    target_bwi = GetBrowserWindowInterface(window_id.value());
+    if (!target_bwi) {
+      // Put it back on the hidden window.
+      source_strip->InsertWebContentsAt(-1, std::move(detached),
+                                        AddTabTypes::ADD_NONE);
+      return Response::ServerError("Browser window not found");
+    }
+  } else {
+    // Find last active non-hidden window.
+    ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+        [this, &target_bwi](BrowserWindowInterface* bwi) {
+          if (!hidden_window_ids_.contains(bwi->GetSessionID().id())) {
+            target_bwi = bwi;
+            return false;
+          }
+          return true;
+        });
+  }
+
+  if (!target_bwi) {
+    Profile* profile =
+        Profile::FromBrowserContext(detached->GetBrowserContext());
+    Browser::CreateParams params(
+        BrowserWindowInterface::TYPE_NORMAL, profile, true);
+    Browser* browser = Browser::Create(params);
+    browser->window()->Show();
+    target_bwi = GetBrowserWindowInterface(browser->session_id().id());
+    if (!target_bwi) {
+      source_strip->InsertWebContentsAt(-1, std::move(detached),
+                                        AddTabTypes::ADD_NONE);
+      return Response::ServerError("Failed to create window for tab");
+    }
+  }
+
+  TabStripModel* tab_strip = target_bwi->GetTabStripModel();
+  int insert_index = index.value_or(tab_strip->count());
+  bool should_activate = activate.value_or(true);
+  int add_types = should_activate ? AddTabTypes::ADD_ACTIVE
+                                  : AddTabTypes::ADD_NONE;
+
+  content::WebContents* raw_wc = detached.get();
+  tab_strip->InsertWebContentsAt(insert_index, std::move(detached),
+                                 add_types);
+
+  int final_index = tab_strip->GetIndexOfWebContents(raw_wc);
+  *out_tab = BuildTabInfo(raw_wc, target_bwi, final_index, false);
+  return Response::Success();
+}
+
+Response BrowserHandler::HideTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  TabLookupResult lookup;
+  Response response = ResolveTabIdentifier(target_id, tab_id,
+                                           hidden_window_ids_, &lookup);
+  if (!response.IsSuccess())
+    return response;
+
+  if (lookup.is_hidden) {
+    return Response::InvalidParams("Tab is already hidden");
+  }
+
+  // Detach from visible window.
+  TabStripModel* source_strip = lookup.bwi->GetTabStripModel();
+  std::unique_ptr<content::WebContents> detached =
+      source_strip->DetachWebContentsAtForInsertion(lookup.tab_index);
+  if (!detached) {
+    return Response::ServerError("Failed to detach tab");
+  }
+
+  // Insert into hidden window.
+  Profile* profile =
+      Profile::FromBrowserContext(detached->GetBrowserContext());
+  Browser* hidden_browser = GetOrCreateHiddenWindow(profile);
+
+  content::WebContents* raw_wc = detached.get();
+  hidden_browser->tab_strip_model()->InsertWebContentsAt(
+      -1, std::move(detached), AddTabTypes::ADD_NONE);
+
+  BrowserWindowInterface* hidden_bwi = GetBrowserWindowInterface(
+      hidden_browser->session_id().id());
+  int new_index =
+      hidden_browser->tab_strip_model()->GetIndexOfWebContents(raw_wc);
+  *out_tab = BuildTabInfo(raw_wc, hidden_bwi, new_index, true);
+  return Response::Success();
+}
+
+// --- Tab Group Management ---
+
+Response BrowserHandler::GetTabGroups(
+    std::optional<int> window_id,
+    std::unique_ptr<protocol::Array<protocol::Browser::TabGroupInfo>>*
+        out_groups) {
+  auto groups =
+      std::make_unique<protocol::Array<protocol::Browser::TabGroupInfo>>();
+
+  if (window_id.has_value()) {
+    BrowserWindowInterface* bwi =
+        GetBrowserWindowInterface(window_id.value());
+    if (!bwi) {
+      return Response::ServerError("Browser window not found");
+    }
+    auto* gm = bwi->GetTabStripModel()->group_model();
+    if (gm) {
+      for (const auto& gid : gm->ListTabGroups()) {
+        groups->push_back(BuildTabGroupInfo(bwi, gid));
+      }
+    }
+  } else {
+    ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+        [&groups](BrowserWindowInterface* bwi) {
+          auto* gm = bwi->GetTabStripModel()->group_model();
+          if (gm) {
+            for (const auto& gid : gm->ListTabGroups()) {
+              groups->push_back(BuildTabGroupInfo(bwi, gid));
+            }
+          }
+          return true;
+        });
+  }
+
+  *out_groups = std::move(groups);
+  return Response::Success();
+}
+
+Response BrowserHandler::CreateTabGroup(
+    std::unique_ptr<protocol::Array<int>> tab_ids,
+    std::optional<std::string> title,
+    std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) {
+  if (!tab_ids || tab_ids->empty()) {
+    return Response::InvalidParams("tabIds must not be empty");
+  }
+
+  BrowserWindowInterface* bwi = nullptr;
+  std::vector<int> indices;
+  Response response = ResolveTabIdsToIndices(*tab_ids, &bwi, &indices);
+  if (!response.IsSuccess()) {
+    return response;
+  }
+
+  TabStripModel* strip = bwi->GetTabStripModel();
+  tab_groups::TabGroupId gid = strip->AddToNewGroup(indices);
+
+  if (title.has_value()) {
+    const tab_groups::TabGroupVisualData* current =
+        strip->group_model()->GetTabGroup(gid)->visual_data();
+    tab_groups::TabGroupVisualData new_data(
+        base::UTF8ToUTF16(title.value()), current->color(),
+        current->is_collapsed());
+    strip->ChangeTabGroupVisuals(gid, new_data);
+  }
+
+  *out_group = BuildTabGroupInfo(bwi, gid);
+  return Response::Success();
+}
+
+Response BrowserHandler::UpdateTabGroup(
+    const std::string& group_id,
+    std::optional<std::string> title,
+    std::optional<std::string> color,
+    std::optional<bool> collapsed,
+    std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) {
+  GroupLookupResult lookup;
+  Response response = ResolveGroupId(group_id, &lookup);
+  if (!response.IsSuccess()) {
+    return response;
+  }
+
+  TabStripModel* strip = lookup.bwi->GetTabStripModel();
+  const tab_groups::TabGroupVisualData* current =
+      strip->group_model()->GetTabGroup(lookup.group_id)->visual_data();
+
+  std::u16string new_title =
+      title.has_value() ? base::UTF8ToUTF16(title.value()) : current->title();
+  tab_groups::TabGroupColorId new_color = current->color();
+  if (color.has_value()) {
+    auto parsed = ParseTabGroupColor(color.value());
+    if (!parsed) {
+      return Response::InvalidParams("Unknown color");
+    }
+    new_color = *parsed;
+  }
+  bool new_collapsed =
+      collapsed.has_value() ? collapsed.value() : current->is_collapsed();
+
+  tab_groups::TabGroupVisualData new_data(new_title, new_color, new_collapsed);
+  strip->ChangeTabGroupVisuals(lookup.group_id, new_data);
+
+  *out_group = BuildTabGroupInfo(lookup.bwi, lookup.group_id);
+  return Response::Success();
+}
+
+Response BrowserHandler::CloseTabGroup(const std::string& group_id) {
+  GroupLookupResult lookup;
+  Response response = ResolveGroupId(group_id, &lookup);
+  if (!response.IsSuccess()) {
+    return response;
+  }
+
+  lookup.bwi->GetTabStripModel()->CloseAllTabsInGroup(lookup.group_id);
+  return Response::Success();
+}
+
+Response BrowserHandler::AddTabsToGroup(
+    const std::string& group_id,
+    std::unique_ptr<protocol::Array<int>> tab_ids,
+    std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) {
+  if (!tab_ids || tab_ids->empty()) {
+    return Response::InvalidParams("tabIds must not be empty");
+  }
+
+  GroupLookupResult lookup;
+  Response response = ResolveGroupId(group_id, &lookup);
+  if (!response.IsSuccess()) {
+    return response;
+  }
+
+  BrowserWindowInterface* bwi = nullptr;
+  std::vector<int> indices;
+  response = ResolveTabIdsToIndices(*tab_ids, &bwi, &indices);
+  if (!response.IsSuccess()) {
+    return response;
+  }
+
+  if (bwi != lookup.bwi) {
+    return Response::InvalidParams(
+        "Tabs must be in the same window as the group");
+  }
+
+  lookup.bwi->GetTabStripModel()->AddToExistingGroup(indices,
+                                                      lookup.group_id);
+
+  *out_group = BuildTabGroupInfo(lookup.bwi, lookup.group_id);
+  return Response::Success();
+}
+
+Response BrowserHandler::RemoveTabsFromGroup(
+    std::unique_ptr<protocol::Array<int>> tab_ids) {
+  if (!tab_ids || tab_ids->empty()) {
+    return Response::InvalidParams("tabIds must not be empty");
+  }
+
+  for (int tid : *tab_ids) {
+    BrowserWindowInterface* found_bwi = nullptr;
+    int found_index = -1;
+
+    ForEachCurrentBrowserWindowInterfaceOrderedByActivation(
+        [tid, &found_bwi, &found_index](BrowserWindowInterface* bwi) {
+          TabStripModel* tab_strip = bwi->GetTabStripModel();
+          for (int i = 0; i < tab_strip->count(); ++i) {
+            content::WebContents* wc = tab_strip->GetWebContentsAt(i);
+            SessionID sid = sessions::SessionTabHelper::IdForTab(wc);
+            if (sid.is_valid() && sid.id() == tid) {
+              found_bwi = bwi;
+              found_index = i;
+              return false;
+            }
+          }
+          return true;
+        });
+
+    if (!found_bwi) {
+      return Response::ServerError("No tab with given id");
+    }
+
+    TabStripModel* strip = found_bwi->GetTabStripModel();
+    if (strip->GetTabGroupForTab(found_index).has_value()) {
+      strip->RemoveFromGroup({found_index});
+    }
+  }
+
+  return Response::Success();
+}
+
+Response BrowserHandler::MoveTabGroup(
+    const std::string& group_id,
+    std::optional<int> window_id,
+    std::optional<int> index,
+    std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) {
+  GroupLookupResult lookup;
+  Response response = ResolveGroupId(group_id, &lookup);
+  if (!response.IsSuccess()) {
+    return response;
+  }
+
+  BrowserWindowInterface* target_bwi = lookup.bwi;
+  if (window_id.has_value()) {
+    target_bwi = GetBrowserWindowInterface(window_id.value());
+    if (!target_bwi) {
+      return Response::ServerError("Browser window not found");
+    }
+  }
+
+  if (target_bwi == lookup.bwi) {
+    // Same-window move.
+    if (index.has_value()) {
+      lookup.bwi->GetTabStripModel()->MoveGroupTo(lookup.group_id,
+                                                   index.value());
+    }
+    *out_group = BuildTabGroupInfo(lookup.bwi, lookup.group_id);
+    return Response::Success();
+  }
+
+  // Cross-window move: collect tabs, detach, re-insert, re-group.
+  TabStripModel* source_strip = lookup.bwi->GetTabStripModel();
+  const tab_groups::TabGroupVisualData* visual =
+      source_strip->group_model()->GetTabGroup(lookup.group_id)->visual_data();
+  tab_groups::TabGroupVisualData saved_visual = *visual;
+
+  // Collect WebContents in the group (reverse order for stable detach).
+  std::vector<content::WebContents*> group_tabs;
+  for (int i = source_strip->count() - 1; i >= 0; --i) {
+    if (source_strip->GetTabGroupForTab(i) == lookup.group_id) {
+      group_tabs.push_back(source_strip->GetWebContentsAt(i));
+    }
+  }
+  std::ranges::reverse(group_tabs);
+
+  // Detach in reverse index order.
+  std::vector<std::unique_ptr<content::WebContents>> detached;
+  for (int i = source_strip->count() - 1; i >= 0; --i) {
+    if (source_strip->GetTabGroupForTab(i) == lookup.group_id) {
+      detached.push_back(
+          source_strip->DetachWebContentsAtForInsertion(i));
+    }
+  }
+  std::ranges::reverse(detached);
+
+  // Insert into target window.
+  TabStripModel* target_strip = target_bwi->GetTabStripModel();
+  int insert_at = index.value_or(target_strip->count());
+  std::vector<int> new_indices;
+  for (auto& wc : detached) {
+    target_strip->InsertWebContentsAt(insert_at, std::move(wc),
+                                      AddTabTypes::ADD_NONE);
+    new_indices.push_back(insert_at);
+    insert_at++;
+  }
+
+  // Group the inserted tabs.
+  tab_groups::TabGroupId new_gid =
+      target_strip->AddToNewGroup(new_indices);
+  target_strip->ChangeTabGroupVisuals(new_gid, saved_visual);
+
+  *out_group = BuildTabGroupInfo(target_bwi, new_gid);
+  return Response::Success();
+}
+
+// --- Hidden Window Helpers ---
+
+Browser* BrowserHandler::GetOrCreateHiddenWindow(Profile* profile) {
+  auto it = hidden_window_per_profile_.find(profile);
+  if (it != hidden_window_per_profile_.end()) {
+    return it->second;
+  }
+
+  Browser::CreateParams params(Browser::TYPE_NORMAL, profile, true);
+  Browser* browser = Browser::Create(params);
+
+  // Add a blank tab so ShowInactive has content to composite.
+  chrome::AddTabAt(browser, GURL(), -1, false);
+  MakeWindowHidden(browser);
+
+  hidden_window_per_profile_[profile] = browser;
+  return browser;
+}
+
+void BrowserHandler::MakeWindowHidden(Browser* browser) {
+#if BUILDFLAG(IS_MAC)
+  SetWindowHeadless(browser->window(), true);
+  browser->window()->ShowInactive();
+#else
+  gfx::Rect offscreen_bounds = browser->window()->GetBounds();
+  offscreen_bounds.set_origin(
+      gfx::Point(kOffScreenPosition, kOffScreenPosition));
+  browser->window()->SetBounds(offscreen_bounds);
+  browser->window()->ShowInactive();
+#endif
+  hidden_window_ids_.insert(browser->session_id().id());
+}
+
+void BrowserHandler::MakeWindowVisible(BrowserWindowInterface* bwi) {
+#if BUILDFLAG(IS_MAC)
+  Browser* browser = bwi->GetBrowserForMigrationOnly();
+  SetWindowHeadless(browser->window(), false);
+#else
+  gfx::Rect bounds = bwi->GetWindow()->GetBounds();
+  bounds.set_origin(gfx::Point(100, 100));
+  bwi->GetWindow()->SetBounds(bounds);
+#endif
+  hidden_window_ids_.erase(bwi->GetSessionID().id());
+}
+
+bool BrowserHandler::IsHiddenWindow(int window_id) const {
+  return hidden_window_ids_.contains(window_id);
+}
+
