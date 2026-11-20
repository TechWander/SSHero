// Preload: expose a narrow, safe bridge on `window.sshero`.
//
// - `isElectron` lets the SPA pick the desktop code path.
// - `getPathForFile(file)` returns the OS absolute path of a dropped File,
//   used to route drag-uploads through `/upload-path` (no bytes cross the
//   webview). Requires Electron 32+.
// - `saveDialog(defaultPath)` opens the native save dialog; the SPA then
//   posts the chosen destination to `/download-to-local`.
// - `env()` / `fileExists(p)` / `pickKeyFile()` back the private-key default
//   path + existence check + native key picker in the session dialog.
// - `getSettings()` / `chooseDataDir()` surface the persisted data directory.

const { contextBridge, ipcRenderer, webUtils } = require('electron')

contextBridge.exposeInMainWorld('sshero', {
  isElectron: true,
  getPathForFile: (file) => {
    try {
      return webUtils.getPathForFile(file)
    } catch (e) {
      console.warn('getPathForFile failed:', e)
      return null
    }
  },
  saveDialog: (defaultPath) => ipcRenderer.invoke('sshero:save-dialog', defaultPath),
  env: () => ipcRenderer.invoke('sshero:env'),
  fileExists: (p) => ipcRenderer.invoke('sshero:file-exists', p),
  pickKeyFile: () => ipcRenderer.invoke('sshero:open-key-dialog'),
  getSettings: () => ipcRenderer.invoke('sshero:get-settings'),
  chooseDataDir: () => ipcRenderer.invoke('sshero:choose-data-dir'),
})
