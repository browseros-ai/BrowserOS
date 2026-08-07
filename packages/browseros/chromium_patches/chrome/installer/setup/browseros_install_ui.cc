diff --git a/chrome/installer/setup/browseros_install_ui.cc b/chrome/installer/setup/browseros_install_ui.cc
new file mode 100644
index 0000000000000000000000000000000000000000..93da7d596dbc87a85996de1b36cf702d94e813fe
--- /dev/null
+++ b/chrome/installer/setup/browseros_install_ui.cc
@@ -0,0 +1,262 @@
+// Copyright 2026 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/installer/setup/browseros_install_ui.h"
+
+#include <windows.h>
+
+#include <commctrl.h>
+
+#include <string>
+#include <utility>
+
+#include "base/logging.h"
+#include "base/strings/strcat_win.h"
+#include "base/synchronization/waitable_event.h"
+#include "chrome/installer/util/install_util.h"
+#include "chrome/installer/util/l10n_string_util.h"
+
+namespace installer {
+
+namespace {
+
+constexpr wchar_t kInstallWindowClass[] = L"BrowserOSInstallProgressWindow";
+constexpr int kWindowWidthDips = 420;
+constexpr int kWindowHeightDips = 150;
+constexpr int kMarginDips = 24;
+constexpr int kTextHeightDips = 24;
+constexpr int kProgressHeightDips = 18;
+constexpr int kProgressTopDips = 76;
+
+LRESULT CALLBACK InstallWindowProc(HWND hwnd,
+                                   UINT message,
+                                   WPARAM wparam,
+                                   LPARAM lparam) {
+  if (message == WM_CLOSE) {
+    return 0;
+  }
+
+  return ::DefWindowProc(hwnd, message, wparam, lparam);
+}
+
+int GetSystemDpi() {
+  HDC screen_dc = ::GetDC(nullptr);
+  if (!screen_dc) {
+    return USER_DEFAULT_SCREEN_DPI;
+  }
+
+  const int dpi = ::GetDeviceCaps(screen_dc, LOGPIXELSX);
+  ::ReleaseDC(nullptr, screen_dc);
+  return dpi > 0 ? dpi : USER_DEFAULT_SCREEN_DPI;
+}
+
+int ScaleForDpi(int dips, int dpi) {
+  return ::MulDiv(dips, dpi, USER_DEFAULT_SCREEN_DPI);
+}
+
+RECT GetCenteredWindowBounds(int width, int height) {
+  RECT work_area = {};
+  if (!::SystemParametersInfo(SPI_GETWORKAREA, 0, &work_area, 0)) {
+    work_area.right = ::GetSystemMetrics(SM_CXSCREEN);
+    work_area.bottom = ::GetSystemMetrics(SM_CYSCREEN);
+  }
+
+  const int x =
+      work_area.left + ((work_area.right - work_area.left - width) / 2);
+  const int y =
+      work_area.top + ((work_area.bottom - work_area.top - height) / 2);
+  return {x, y, x + width, y + height};
+}
+
+std::wstring GetWindowTitle(const std::wstring& display_name) {
+  return base::StrCat({display_name, L" Installer"});
+}
+
+std::wstring GetInstallMessage(const std::wstring& display_name) {
+  return base::StrCat({L"Installing ", display_name, L"..."});
+}
+
+std::wstring GetFailureMessage(int install_msg_base,
+                               const std::wstring& display_name) {
+  if (install_msg_base != 0) {
+    std::wstring message = installer::GetLocalizedString(install_msg_base);
+    if (!message.empty()) {
+      return message;
+    }
+  }
+
+  return base::StrCat({L"Failed to install ", display_name, L"."});
+}
+
+}  // namespace
+
+class BrowserOSInstallUI::UIThreadDelegate final
+    : public base::PlatformThread::Delegate {
+ public:
+  explicit UIThreadDelegate(std::wstring display_name)
+      : display_name_(std::move(display_name)) {}
+
+  UIThreadDelegate(const UIThreadDelegate&) = delete;
+  UIThreadDelegate& operator=(const UIThreadDelegate&) = delete;
+
+  void ThreadMain() override {
+    base::PlatformThread::SetName("Installer UI");
+    thread_id_ = ::GetCurrentThreadId();
+
+    MSG msg;
+    ::PeekMessage(&msg, nullptr, WM_USER, WM_USER, PM_NOREMOVE);
+
+    const INITCOMMONCONTROLSEX init_controls = {
+        .dwSize = sizeof(INITCOMMONCONTROLSEX),
+        .dwICC = ICC_PROGRESS_CLASS,
+    };
+    if (!::InitCommonControlsEx(&init_controls)) {
+      LOG(ERROR) << "Failed to initialize installer progress control: "
+                 << ::GetLastError();
+      ready_event_.Signal();
+      return;
+    }
+
+    const HINSTANCE instance = ::GetModuleHandle(nullptr);
+    WNDCLASSEX window_class = {
+        .cbSize = sizeof(WNDCLASSEX),
+        .lpfnWndProc = InstallWindowProc,
+        .hInstance = instance,
+        .hCursor = ::LoadCursor(nullptr, IDC_ARROW),
+        .hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1),
+        .lpszClassName = kInstallWindowClass,
+    };
+
+    if (!::RegisterClassEx(&window_class) &&
+        ::GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
+      LOG(ERROR) << "Failed to register installer progress window: "
+                 << ::GetLastError();
+      ready_event_.Signal();
+      return;
+    }
+
+    const int dpi = GetSystemDpi();
+    RECT bounds = GetCenteredWindowBounds(ScaleForDpi(kWindowWidthDips, dpi),
+                                          ScaleForDpi(kWindowHeightDips, dpi));
+    const std::wstring title = GetWindowTitle(display_name_);
+    hwnd_ = ::CreateWindowEx(WS_EX_DLGMODALFRAME | WS_EX_APPWINDOW,
+                             kInstallWindowClass, title.c_str(),
+                             WS_CAPTION | WS_POPUP, bounds.left, bounds.top,
+                             bounds.right - bounds.left,
+                             bounds.bottom - bounds.top, nullptr, nullptr,
+                             instance, nullptr);
+    if (!hwnd_) {
+      LOG(ERROR) << "Failed to create installer progress window: "
+                 << ::GetLastError();
+      ready_event_.Signal();
+      return;
+    }
+
+    const HFONT font =
+        reinterpret_cast<HFONT>(::GetStockObject(DEFAULT_GUI_FONT));
+    const int margin = ScaleForDpi(kMarginDips, dpi);
+    const int content_width = bounds.right - bounds.left - (2 * margin);
+
+    const std::wstring install_message = GetInstallMessage(display_name_);
+    HWND text = ::CreateWindowEx(
+        0, L"STATIC", install_message.c_str(), WS_CHILD | WS_VISIBLE | SS_LEFT,
+        margin, margin, content_width, ScaleForDpi(kTextHeightDips, dpi), hwnd_,
+        nullptr, instance, nullptr);
+    if (text && font) {
+      ::SendMessage(text, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
+    }
+
+    HWND progress = ::CreateWindowEx(
+        0, PROGRESS_CLASS, nullptr, WS_CHILD | WS_VISIBLE | PBS_MARQUEE, margin,
+        ScaleForDpi(kProgressTopDips, dpi), content_width,
+        ScaleForDpi(kProgressHeightDips, dpi), hwnd_, nullptr, instance,
+        nullptr);
+    if (progress) {
+      ::SendMessage(progress, PBM_SETMARQUEE, TRUE, 0);
+    }
+
+    ::ShowWindow(hwnd_, SW_SHOWNORMAL);
+    ::UpdateWindow(hwnd_);
+    ready_event_.Signal();
+
+    while (::GetMessage(&msg, nullptr, 0, 0) > 0) {
+      ::TranslateMessage(&msg);
+      ::DispatchMessage(&msg);
+    }
+
+    if (hwnd_) {
+      ::DestroyWindow(hwnd_);
+      hwnd_ = nullptr;
+    }
+  }
+
+  void WaitUntilReady() { ready_event_.Wait(); }
+
+  void RequestClose() {
+    if (thread_id_ != 0) {
+      ::PostThreadMessage(thread_id_, WM_QUIT, 0, 0);
+    }
+  }
+
+ private:
+  const std::wstring display_name_;
+  base::WaitableEvent ready_event_{
+      base::WaitableEvent::ResetPolicy::MANUAL,
+      base::WaitableEvent::InitialState::NOT_SIGNALED};
+  DWORD thread_id_ = 0;
+  HWND hwnd_ = nullptr;
+};
+
+BrowserOSInstallUI::BrowserOSInstallUI(bool should_show)
+    : should_show_(should_show) {}
+
+BrowserOSInstallUI::~BrowserOSInstallUI() {
+  Close();
+}
+
+void BrowserOSInstallUI::Show() {
+  if (!should_show_ || ui_thread_) {
+    return;
+  }
+
+  ui_thread_ =
+      std::make_unique<UIThreadDelegate>(InstallUtil::GetDisplayName());
+  if (!base::PlatformThread::Create(0, ui_thread_.get(), &thread_handle_)) {
+    LOG(ERROR) << "Failed to start installer progress UI thread";
+    ui_thread_.reset();
+    return;
+  }
+
+  ui_thread_->WaitUntilReady();
+}
+
+void BrowserOSInstallUI::Close() {
+  if (!ui_thread_) {
+    return;
+  }
+
+  ui_thread_->RequestClose();
+  base::PlatformThread::Join(thread_handle_);
+  thread_handle_ = base::PlatformThreadHandle();
+  ui_thread_.reset();
+}
+
+void BrowserOSInstallUI::CloseAndShowFailureMessage(int install_msg_base) {
+  if (!should_show_) {
+    return;
+  }
+
+  Close();
+  ShowBrowserOSInstallFailureMessageBox(install_msg_base);
+}
+
+void ShowBrowserOSInstallFailureMessageBox(int install_msg_base) {
+  const std::wstring display_name = InstallUtil::GetDisplayName();
+  const std::wstring title = GetWindowTitle(display_name);
+  const std::wstring message = GetFailureMessage(install_msg_base, display_name);
+  ::MessageBox(nullptr, message.c_str(), title.c_str(),
+               MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TASKMODAL);
+}
+
+}  // namespace installer
