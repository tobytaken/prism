import { app, BrowserWindow, shell } from "electron";
import path from "node:path";

import { registerIpc } from "./ipc";
import { registerAppProtocol } from "./protocol";

let mainWindow: BrowserWindow | null = null;
let disposeIpc: (() => void) | null = null;

function isAllowedExternalUrl(url: string) {
  return url.startsWith("https://") || url.startsWith("mailto:");
}

app.on("web-contents-created", (_event, contents) => {
  contents.on("will-navigate", (event, url) => {
    const currentUrl = contents.getURL();
    if (url === currentUrl) {
      return;
    }

    event.preventDefault();
    if (isAllowedExternalUrl(url)) {
      void shell.openExternal(url);
    }
  });

  contents.setWindowOpenHandler(({ url }) => {
    if (isAllowedExternalUrl(url)) {
      void shell.openExternal(url);
    }
    return { action: "deny" };
  });

  contents.session.setPermissionRequestHandler(
    (_webContents, _permission, callback) => {
      callback(false);
    },
  );
});

function emitWindowState() {
  if (!mainWindow) {
    return;
  }

  mainWindow.webContents.send("prism:window:resized", mainWindow.isMaximized());
}

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1320,
    height: 860,
    minWidth: 1040,
    minHeight: 680,
    frame: false,
    transparent: false,
    backgroundColor: "#0b0e12",
    title: "Prism Electron",
    show: false,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
    },
  });

  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (isAllowedExternalUrl(url)) {
      void shell.openExternal(url);
    }
    return { action: "deny" };
  });

  mainWindow.webContents.on("will-navigate", (event, url) => {
    if (url === mainWindow?.webContents.getURL()) {
      return;
    }

    event.preventDefault();
    if (isAllowedExternalUrl(url)) {
      void shell.openExternal(url);
    }
  });

  mainWindow.once("ready-to-show", () => {
    mainWindow?.show();
    emitWindowState();
  });

  mainWindow.on("resize", emitWindowState);
  mainWindow.on("maximize", emitWindowState);
  mainWindow.on("unmaximize", emitWindowState);
  mainWindow.on("closed", () => {
    mainWindow = null;
  });

  if (MAIN_WINDOW_VITE_DEV_SERVER_URL) {
    void mainWindow.loadURL(MAIN_WINDOW_VITE_DEV_SERVER_URL);
  } else {
    void mainWindow.loadURL("app://bundle/index.html");
  }
}

app.on("ready", () => {
  registerAppProtocol(MAIN_WINDOW_VITE_NAME);
  disposeIpc = registerIpc(() => mainWindow);
  createWindow();
});

app.on("window-all-closed", () => {
  app.quit();
});

app.on("before-quit", () => {
  disposeIpc?.();
  disposeIpc = null;
});

app.on("activate", () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow();
  }
});
