// Smoke-check: launch Electron, wait for the SPA to render, save a PNG, quit.
// Usage: SSHERO_DATA_DIR=... xvfb-run -a electron screenshot.cjs

const { app, BrowserWindow } = require('electron')
require('./main.cjs')

async function grab() {
  // give the renderer time to boot
  await new Promise((r) => setTimeout(r, 3500))
  const win = BrowserWindow.getAllWindows()[0]
  if (!win) { console.error('no window'); app.exit(1); return }
  const img = await win.webContents.capturePage()
  require('node:fs').writeFileSync(process.env.OUT_PNG || '/tmp/sshero-electron.png', img.toPNG())
  console.log('screenshot saved')
  app.exit(0)
}

app.whenReady().then(() => setTimeout(grab, 200))
