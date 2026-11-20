// Electron main process: spawn the Rust sshero backend as a child,
// hand it a random token, wait for its ready file, then open a window
// on http://127.0.0.1:<port>/?token=<t>.
//
// Only loopback; only this process can reach the backend (random port
// + random token + no CORS preselection).

const { app, BrowserWindow, dialog, ipcMain } = require('electron')
const { spawn } = require('node:child_process')
const crypto = require('node:crypto')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')

// ---------- locate the Rust backend binary ----------
function findBackend() {
  const ext = process.platform === 'win32' ? '.exe' : ''
  if (process.env.SSHERO_BACKEND_BIN && fs.existsSync(process.env.SSHERO_BACKEND_BIN)) {
    return process.env.SSHERO_BACKEND_BIN
  }
  // packaged install: electron-builder places extraResources at process.resourcesPath;
  // prefer the platform-arch variant (mac arm64 vs x64), then the generic name
  if (process.resourcesPath) {
    const bin = path.join(process.resourcesPath, 'bin')
    const variant = path.join(bin, `sshero-backend-${process.platform}-${process.arch}${ext}`)
    if (fs.existsSync(variant)) return variant
    const generic = path.join(bin, `sshero-backend${ext}`)
    if (fs.existsSync(generic)) return generic
  }
  const repo = path.resolve(__dirname, '../..')
  const cands = [
    path.join(repo, `target/release/sshero-backend${ext}`),
    path.join(repo, `target/debug/sshero-backend${ext}`),
  ]
  for (const p of cands) if (fs.existsSync(p)) return p
  throw new Error(
    'sshero-backend binary not found — run `cargo build --release` first, or set SSHERO_BACKEND_BIN',
  )
}

function findStatic() {
  // packaged: SPA is unpacked to resources/frontend-dist — a plain dir, because
  // the Rust backend process cannot read files inside the electron asar archive
  if (process.resourcesPath) {
    const p = path.join(process.resourcesPath, 'frontend-dist')
    if (fs.existsSync(path.join(p, 'index.html'))) return p
  }
  const repo = path.resolve(__dirname, '../..')
  const dist = path.join(repo, 'frontend/dist')
  return fs.existsSync(path.join(dist, 'index.html')) ? dist : undefined
}

// ---------- persisted shell settings (data dir override) ----------
// A tiny JSON file in userData lets the user relocate where sessions/known_hosts
// live without env vars. Read once at startup; the backend is spawned with
// --data-dir pointing at it. Changing it takes effect on next launch.
function settingsPath() {
  return path.join(app.getPath('userData'), 'settings.json')
}
function loadSettings() {
  try {
    return JSON.parse(fs.readFileSync(settingsPath(), 'utf8')) || {}
  } catch {
    return {}
  }
}
function saveSettings(s) {
  try {
    fs.mkdirSync(path.dirname(settingsPath()), { recursive: true })
    fs.writeFileSync(settingsPath(), JSON.stringify(s, null, 2))
    return true
  } catch (e) {
    console.error('failed to persist settings:', e)
    return false
  }
}
function defaultDataDir() {
  return path.join(os.homedir(), '.local', 'share', 'sshero')
}
function currentDataDir() {
  const s = loadSettings()
  return s.dataDir || process.env.SSHERO_DATA_DIR || defaultDataDir()
}

// ---------- backend lifecycle ----------
let backend = null
let info = null

async function startBackend() {
  const bin = findBackend()
  const token = crypto.randomBytes(32).toString('hex')
  const readyFile = path.join(os.tmpdir(), `sshero-ready-${process.pid}.json`)
  try { fs.unlinkSync(readyFile) } catch { /* ok */ }
  const dataDir = currentDataDir()
  fs.mkdirSync(dataDir, { recursive: true })

  const args = [
    '--listen', '127.0.0.1:0',
    '--token', token,
    '--data-dir', dataDir,
    '--ready-file', readyFile,
  ]
  const staticDir = findStatic()
  if (staticDir) args.push('--static', staticDir)

  backend = spawn(bin, args, { stdio: ['ignore', 'pipe', 'pipe'] })
  backend.stdout.on('data', (d) => process.stderr.write(`[backend] ${d}`))
  backend.stderr.on('data', (d) => process.stderr.write(`[backend] ${d}`))
  backend.on('exit', (code, sig) => {
    console.error(`backend exited code=${code} sig=${sig}`)
    backend = null
  })

  const deadline = Date.now() + 8000
  while (!fs.existsSync(readyFile)) {
    if (Date.now() > deadline) throw new Error('backend did not become ready in 8s')
    if (!backend) throw new Error('backend exited during startup')
    await new Promise((r) => setTimeout(r, 60))
  }
  const parsed = JSON.parse(fs.readFileSync(readyFile, 'utf8'))
  try { fs.unlinkSync(readyFile) } catch { /* ok */ }
  info = { port: parsed.port, token }
  return info
}

function stopBackend() {
  if (backend) {
    try { backend.kill('SIGTERM') } catch { /* ok */ }
    backend = null
  }
}

// ---------- window ----------
async function createWindow() {
  await startBackend()
  const win = new BrowserWindow({
    width: 1280,
    height: 800,
    minWidth: 900,
    minHeight: 560,
    title: 'SSH 文件管理器',
    backgroundColor: '#0f1419',
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  })
  win.loadURL(`http://127.0.0.1:${info.port}/?token=${info.token}`)
  win.on('closed', () => app.quit())
}

// ---------- IPC ----------
ipcMain.handle('sshero:save-dialog', async (_e, defaultPath) => {
  const r = await dialog.showSaveDialog({ defaultPath, title: '保存到本地' })
  return r.canceled ? null : r.filePath
})

// environment info the SPA needs to prefill a sensible default private-key path
ipcMain.handle('sshero:env', () => ({
  platform: process.platform,
  home: os.homedir(),
}))

// does a local path exist (used to auto-load a detected key file)
ipcMain.handle('sshero:file-exists', (_e, p) => {
  try { return typeof p === 'string' && p.length > 0 && fs.statSync(p).isFile() } catch { return false }
})

// native file picker for a private key
ipcMain.handle('sshero:open-key-dialog', async () => {
  const r = await dialog.showOpenDialog({
    title: '选择私钥文件',
    properties: ['openFile'],
    defaultPath: path.join(os.homedir(), '.ssh'),
    filters: [
      { name: '私钥', extensions: ['pem', 'pub', 'key', 'ed25519', 'rsa', 'dss', 'ecdsa'] },
      { name: '所有文件', extensions: ['*'] },
    ],
  })
  return r.canceled ? null : r.filePaths[0]
})

// shell settings: current + default data dir (surfaced in the Settings dialog)
ipcMain.handle('sshero:get-settings', () => ({
  dataDir: currentDataDir(),
  defaultDataDir: defaultDataDir(),
  platform: process.platform,
}))

ipcMain.handle('sshero:choose-data-dir', async () => {
  const r = await dialog.showOpenDialog({
    title: '选择数据目录（会话档案 / known_hosts 的存放位置）',
    properties: ['openDirectory', 'createDirectory'],
    defaultPath: currentDataDir(),
  })
  if (r.canceled || !r.filePaths[0]) return null
  const s = loadSettings()
  s.dataDir = r.filePaths[0]
  saveSettings(s)
  return { dataDir: s.dataDir }
})

// ---------- app lifecycle ----------
app.whenReady().then(createWindow).catch((e) => {
  console.error('startup failed:', e)
  dialog.showErrorBox('SSH 文件管理器启动失败', String((e && e.message) || e))
  app.exit(1)
})

app.on('window-all-closed', () => {
  stopBackend()
  if (process.platform !== 'darwin') app.quit()
})
app.on('before-quit', stopBackend)
