/// Desktop (Tauri / Electron) ↔ browser bridge.
///
/// The same SPA runs inside either desktop shell or in a plain browser:
/// * Both shells hand the backend token to the SPA via `?token=` on the
///   window URL (random loopback port + random token, per-launch).
/// * Tauri emits `tauri://drag-drop` events carrying **absolute local paths**,
///   forwarded to `/upload-path`.
/// * Electron provides `window.sshero.getPathForFile(file)` (Electron 32+),
///   so HTML5 `drop` events with `DataTransfer.files` are turned into OS
///   paths and also routed through `/upload-path` — no bytes cross the
///   webview.
/// * Downloads use the native save dialog when available
///   (`window.sshero.saveDialog` / `@tauri-apps/plugin-dialog`) and then
///   `/download-to-local`; browsers fall back to the anchor-blob path.

export function isTauri(): boolean {
  return typeof (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ !== 'undefined'
}

interface ElectronBridge {
  isElectron: true
  getPathForFile(file: File): string | null
  saveDialog(defaultPath?: string): Promise<string | null>
  env(): Promise<{ platform: string; home: string }>
  fileExists(p: string): Promise<boolean>
  pickKeyFile(): Promise<string | null>
  getSettings(): Promise<{ dataDir: string; defaultDataDir: string; platform: string }>
  chooseDataDir(): Promise<{ dataDir: string } | null>
}

export function electronBridge(): ElectronBridge | null {
  const b = (window as unknown as { sshero?: ElectronBridge }).sshero
  return b && b.isElectron ? b : null
}

export function isElectron(): boolean {
  return electronBridge() !== null
}

/** true when running under any desktop shell (native drop/save supported) */
export function isDesktop(): boolean {
  return isTauri() || isElectron()
}

/** read ?token= once (desktop shell hands it in the URL) and persist it */
export function initToken(): void {
  const t = new URLSearchParams(location.search).get('token')
  if (t) localStorage.setItem('sshero.token', t)
}

// ---------- private-key default path (session dialog) ----------

/**
 * Best-effort current OS. Electron asks the main process (authoritative);
 * otherwise infer from the UA so a browser still prefills a plausible hint.
 */
export async function detectPlatform(): Promise<'win32' | 'darwin' | 'linux'> {
  const b = electronBridge()
  if (b) {
    try {
      const e = await b.env()
      if (e.platform === 'win32') return 'win32'
      if (e.platform === 'darwin') return 'darwin'
      return 'linux'
    } catch {
      /* fall through */
    }
  }
  const ua = navigator.userAgent
  if (/Windows/i.test(ua)) return 'win32'
  if (/Mac|iPhone|iPad/i.test(ua)) return 'darwin'
  return 'linux'
}

/**
 * Candidate absolute private-key paths for the current OS, most-preferred
 * first (ed25519 then rsa). Returns [] when we cannot know the home directory
 * (plain browser) — the caller then leaves the field for manual entry.
 */
export async function defaultKeyPaths(): Promise<string[]> {
  const b = electronBridge()
  const plat = await detectPlatform()
  let home: string | null = null
  if (b) {
    try {
      home = (await b.env()).home
    } catch {
      home = null
    }
  }
  if (!home) return []
  const names = ['id_ed25519', 'id_rsa']
  if (plat === 'win32') return names.map((n) => `${home}\\.ssh\\${n}`)
  return names.map((n) => `${home}/.ssh/${n}`)
}

/** true only when Electron confirms the file exists; unknown elsewhere */
export async function localFileExists(p: string): Promise<boolean> {
  const b = electronBridge()
  if (!b) return false
  try {
    return await b.fileExists(p)
  } catch {
    return false
  }
}

/** native private-key picker; returns null when unsupported or canceled */
export async function pickKeyFile(): Promise<string | null> {
  const b = electronBridge()
  if (!b) return null
  try {
    return await b.pickKeyFile()
  } catch {
    return null
  }
}

// ---------- shell settings (data directory) ----------
export interface ShellSettings {
  dataDir: string
  defaultDataDir: string
  platform: string
}

export async function getSettings(): Promise<ShellSettings | null> {
  const b = electronBridge()
  if (!b) return null
  try {
    return await b.getSettings()
  } catch {
    return null
  }
}

/** opens a native directory picker and persists the choice; null if unchanged */
export async function chooseDataDir(): Promise<string | null> {
  const b = electronBridge()
  if (!b) return null
  try {
    const r = await b.chooseDataDir()
    return r ? r.dataDir : null
  } catch {
    return null
  }
}

export interface DragDropPayload {
  paths: string[]
  position: { x: number; y: number }
}

/**
 * Subscribe to Tauri native file drops (OS → webview); cb receives absolute
 * paths. Returns an unsubscribe function. No-op outside Tauri (Electron
 * handles drops via HTML5 in the FilesTab, translating each File to a path).
 */
export function onDragDrop(cb: (payload: DragDropPayload) => unknown): () => void {
  if (!isTauri()) return () => undefined
  let unlisten: (() => void) | undefined
  let disposed = false
  const pending = import('@tauri-apps/api/window')
    .then(({ getCurrentWindow }) =>
      getCurrentWindow().onDragDropEvent((event) => {
        if (event.payload.type === 'drop' || event.payload.type === 'enter') {
          cb({ paths: event.payload.paths, position: event.payload.position })
        }
      }),
    )
    .then((fn) => {
      if (disposed) fn()
      else unlisten = fn
    })
    .catch((e) => console.warn('drag-drop hook unavailable:', e))
  return () => {
    disposed = true
    if (unlisten) unlisten()
    else void pending
  }
}

/**
 * Electron: translate a dropped File list into absolute OS paths via the
 * preload bridge. Returns null if any file cannot be resolved.
 */
export function electronPathsForFiles(files: File[]): string[] | null {
  const b = electronBridge()
  if (!b) return null
  const out: string[] = []
  for (const f of files) {
    const p = b.getPathForFile(f)
    if (!p) return null
    out.push(p)
  }
  return out
}

/**
 * Desktop: native save dialog → stream remote file/dir to that local path.
 * Outcomes:
 *  - 'unsupported': not running under any desktop shell → caller falls back
 *    to browser blob download.
 *  - 'canceled': user dismissed the dialog.
 *  - {savedTo}: file successfully written to that absolute local path.
 */
export type SaveOutcome = 'unsupported' | 'canceled' | { savedTo: string }

export async function saveAndDownload(
  suggestedName: string,
  run: (dest: string) => Promise<unknown>,
): Promise<SaveOutcome> {
  const eb = electronBridge()
  if (eb) {
    const dest = await eb.saveDialog(suggestedName)
    if (!dest) return 'canceled'
    await run(dest)
    return { savedTo: dest }
  }
  if (!isTauri()) return 'unsupported'
  const { save } = await import('@tauri-apps/plugin-dialog')
  const dest = await save({ defaultPath: suggestedName, title: '保存到本地' })
  if (!dest) return 'canceled'
  await run(dest)
  return { savedTo: dest }
}
