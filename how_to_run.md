# How to Run Shimmy-Browser (Extension + Server)

<div align="center">
  <img src="docs/images/shimmy-wordmark.png" width="280" height="70" alt="Shimmy-Browser logo" />
</div>

Welcome to the development guide for **Shimmy-Browser**! This document provides a clean, step-by-step walkthrough to get the development stack running on your machine and publish your changes to GitHub.

---

## 🛠️ Prerequisites

Before you start, make sure you have the following installed:

*   **[Bun](https://bun.sh)** (v1.3.6 or newer) – Used for package management and running the server/scripts.
*   **Git** – For version control and managing the submodules.
*   **Google Chrome** or **Microsoft Edge** – Already installed on your computer.

---

## 🚀 Quick Start Setup (Step-by-Step)

Follow these steps to go from a fresh repository clone to a running browser development environment.

### Step 1: Clone the Repository & Submodules
Clone the repository and initialize the **Sup-agent** submodule:

```bash
# Clone the repository (replace with your fork's URL if applicable)
git clone https://github.com/Karthikprasadm/Shimmy-Browser.git
cd Shimmy-Browser

# Initialize and update the submodules
git submodule update --init --recursive
```

### Step 2: Install Dependencies
Navigate to the agent monorepo directory and install the required npm packages:

```bash
cd packages/browseros-agent
bun install
```

### Step 3: Configure the Environment (`.env.development`)
Copy the environment example template to create your local development configuration:

*   **Bash (macOS / Linux / Git Bash on Windows):**
    ```bash
    cp .env.development.example .env.development
    ```
*   **PowerShell (Windows):**
    ```powershell
    Copy-Item .env.development.example .env.development
    ```

---

## 🌐 Running Without Installing/Building Chromium

> [!TIP]
> **You do not need to download or build a custom Chromium binary.** You can use your already-installed system Google Chrome or Microsoft Edge.

Open the new **`packages/browseros-agent/.env.development`** file you created in Step 3 and set the **`BROWSEROS_BINARY`** variable to your system's browser executable path.

### Example Paths for `BROWSEROS_BINARY`:

*   **Google Chrome:**
    *   **Windows (System):** `BROWSEROS_BINARY=C:\Program Files\Google\Chrome\Application\chrome.exe`
    *   **Windows (User):** `BROWSEROS_BINARY=C:\Users\<YourUsername>\AppData\Local\Google\Chrome\Application\chrome.exe`
    *   **macOS:** `BROWSEROS_BINARY=/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`
    *   **Linux:** `BROWSEROS_BINARY=/usr/bin/google-chrome`
*   **Microsoft Edge:**
    *   **Windows:** `BROWSEROS_BINARY=C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`
    *   **macOS:** `BROWSEROS_BINARY=/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge`

> [!IMPORTANT]
> Verify that the browser executable actually exists at your configured path. WXT will launch this browser with the unpacked extension preloaded when you run the development stack.

---

## 🏃 Running the Development Stack

To start the full development environment (which compiles the extension, opens the browser, and starts the server in the correct order):

From the **repository root** (`Shimmy-Browser`) OR `packages/browseros-agent`:
```bash
bun run dev
```

### What happens behind the scenes?
1. The script loads variables from your `.env.development`.
2. It starts the extension compiler and opens your configured browser with the extension pre-loaded.
3. It waits for the browser's remote debugging interface (CDP) to become responsive.
4. It starts the Bun MCP server, which connects to the browser.

---

## 📋 Command Cheat Sheet

All commands below should be run inside `packages/browseros-agent` unless noted otherwise:

| Command | Working Directory | Effect |
| :--- | :--- | :--- |
| `bun run dev` | Repo Root / `packages/browseros-agent` | **Recommended:** Starts full stack (Extension + Browser + Server) |
| `bun run start:agent` | `packages/browseros-agent` | Launches the browser with the Extension UI only |
| `bun run start:server` | `packages/browseros-agent` | Launches the Bun MCP server only (Browser must be running) |
| `bun run lint:fix` | `packages/browseros-agent` | Automatically formats and fixes code issues with Biome |
| `bun install` | `packages/browseros-agent` | Resolves and installs packages for all monorepo apps |

---

## 🛠️ Verification & Health Checks

You can check if the components are running correctly by calling these endpoints in another terminal:

*   **Server Health Check:**
    ```bash
    curl -s http://127.0.0.1:9111/health
    ```
    *Should return: `{"status":"ok","cdpConnected":true}`*

*   **CDP Browser Debugging Check:**
    ```bash
    curl -s http://127.0.0.1:9333/json/version
    ```
    *Should return details about the running browser.*

---

## 🛑 Stopping the Stack
To stop the entire running stack, press **`Ctrl + C`** in the terminal where the command is running. The runner script will safely terminate both the browser and server processes.

---

## 💻 VS Code Integration
If you use VS Code, you can start the full stack directly inside the editor:
1. Open the Command Palette (`Ctrl+Shift+P` / `Cmd+Shift+P`).
2. Run **Tasks: Run Task**.
3. Select **`Dev: full stack (agent + browser + server)`**.

---

## 📤 Pushing Changes & Submodule Workflows

### Commit Guidelines
We use **Conventional Commits** for commit messages. Ensure your commit follows this format:
```text
<type>(<scope>): <short description>
```
*Examples:*
*   `feat(agent): improve startup time`
*   `fix(server): align CDP connection port`
*   `docs: update running instructions`

---

### Managing Submodule Changes (Sup-agent)
Since `packages/browseros-agent/vendor/sup-agent` is a separate git repository, follow these steps if you make changes inside it:

#### 1. Commit and push inside the submodule directory first:
```bash
cd packages/browseros-agent/vendor/sup-agent
git add .
git commit -m "feat: my submodule changes"
git push origin <your-branch>
```

#### 2. Commit the updated submodule pointer in the main repository:
```bash
# Return to the repository root
cd ../../../../..

# Stage and commit the updated submodule pointer reference
git add packages/browseros-agent/vendor/sup-agent
git commit -m "chore: bump sup-agent submodule pointer"
git push origin <your-branch>
```

---

## 🔍 Troubleshooting Common Issues

| Issue | Cause | Solution |
| :--- | :--- | :--- |
| **"Connect failed / Failed to fetch"** in the UI | Ports in the code and `.env.development` do not match. | Verify `BROWSEROS_SERVER_PORT` and `VITE_PUBLIC_BROWSEROS_API` are consistent in your `.env.development`. |
| **CDP disconnected / bad health** | Browser was closed, or the CDP port matches another process. | Close any stale browser instances and restart using `bun run dev`. |
| **Blank extension page** | Build files are missing or incomplete. | Run `bun install` inside `packages/browseros-agent` and restart the stack. |
| **Submodule folder is empty** | Submodule was not initialized during clone. | Run `git submodule update --init --recursive` at the root. |
| **Commit rejected by hooks** | Biome lint errors or incorrect commit format. | Run `bun run lint:fix` to auto-fix code issues, and format your commit message using Conventional Commits. |
