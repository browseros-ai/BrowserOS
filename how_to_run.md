# How to Run Shimmy Browser (BrowserOS) Local Development Environment

This guide explains how to properly configure, run, and troubleshoot the Shimmy Browser project locally.

---

## 📋 Prerequisites

Before running the project, ensure you have the following installed:
1. **Bun**: The primary package manager and runtime used in this project.
2. **BrowserOS Chromium**: A custom Chromium fork that contains the necessary BrowserOS-specific APIs and permissions (like `"browserOS"`).
   * **Default Windows Path:** `C:\Users\<Your_Username>\AppData\Local\Chromium\Application\chrome.exe`

---

## ⚙️ Environment Configuration

For the development stack to run, you must configure the environment variables and ports correctly.

### 1. The Environment Files (`.env.development`)
There are three synced `.env.development` files in the repository that you need to configure (copying from `.env.development.example`):
* `packages/browseros-agent/.env.development`
* `packages/browseros-agent/apps/app/.env.development`
* `packages/browseros-agent/apps/server/.env.development`

Make sure **`BROWSEROS_BINARY`** points to the custom Chromium build path (use forward slashes `/`):
```env
# Example for Windows:
BROWSEROS_BINARY=C:/Users/<Your_Username>/AppData/Local/Chromium/Application/chrome.exe

# Ports configuration
BROWSEROS_CDP_PORT=9005
BROWSEROS_SERVER_PORT=9105
BROWSEROS_EXTENSION_PORT=9305
```

### 2. The Sidecar Configuration (`config.dev.json`)
The BrowserOS Server reads its ports directly from the sidecar JSON file located at:
`packages/browseros-agent/config.dev.json`

Ensure the ports in this JSON file match the ports defined in your `.env.development` files:
```json
{
  "ports": {
    "server": 9105,
    "cdp": 9005,
    "proxy": 9105
  },
  "directories": {
    "resources": "./resources",
    "execution": "./out"
  },
  "flags": {
    "allow_remote_in_mcp": false
  }
}
```

---

## 🚀 How to Run

1. Open a PowerShell/Terminal window.
2. Navigate to the root directory of the workspace:
   ```powershell
   cd D:\Shimmy-Browser
   ```
3. Run the development stack:
   ```powershell
   bun run dev
   ```

---

## 🛠️ How it Works & Troubleshooting

### Automatic Process Cleanup
If you exit the development stack or it crashes, Chromium background processes might stay running and lock the remote debugging port `9005` or your user data profile.
* **Solution:** The stack runner (`stack.ts`) automatically cleans up any dangling custom Chromium instances running from `AppData/Local/Chromium` at startup.

### Seamless Startup (Blank Page Redirect)
Starting Chromium directly with `--app=chrome-extension://...` on the command line often causes a race condition where Chromium loads the URL before it has finished registering the unpacked extension directory, resulting in an `ERR_BLOCKED_BY_CLIENT` page.
* **Solution:** The stack runner launches Chromium pointing to `about:blank`, waits until the Chrome DevTools Protocol (CDP) port is open and the extension is fully registered, and then automatically redirects the window to the correct extension URL.

---

## 📂 Project Structure

* **`packages/browseros-agent/scripts/dev/stack.ts`**: The orchestrator script that starts WXT, launches the custom Chromium browser, waits for CDP, and spawns the backend server.
* **`packages/browseros-agent/apps/app/`**: The frontend Chrome extension.
* **`packages/browseros-agent/apps/server/`**: The backend MCP endpoints and orchestrator server.
