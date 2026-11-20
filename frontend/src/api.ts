/// Typed client for the sshero-backend REST API.
const TOKEN_KEY = 'sshero.token'

export function getToken(): string {
  return localStorage.getItem(TOKEN_KEY) ?? ''
}
export function setToken(t: string) {
  if (t) localStorage.setItem(TOKEN_KEY, t)
  else localStorage.removeItem(TOKEN_KEY)
}

export type AuthMethod =
  | { kind: 'password'; password: string }
  | { kind: 'key'; key_path: string; key_passphrase?: string | null }

export interface ConnectParams {
  host: string
  port: number
  username: string
  auth: AuthMethod
}

export interface ConnectionInfo {
  id: string
  host: string
  port: number
  username: string
  auth_kind: 'password' | 'key'
  connected_at: number
}

export interface RemoteEntry {
  name: string
  path: string
  is_dir: boolean
  is_symlink: boolean
  size: number
  mtime: number
  mode: number
  uid: number | null
  gid: number | null
}

export interface ListResponse {
  path: string
  entries: RemoteEntry[]
}

// ---------- persisted session profiles (sidebar) ----------
export type SessionAuthKind = 'password' | 'key-path' | 'key-content'

export interface SessionMeta {
  id: string
  name: string
  host: string
  port: number
  username: string
  auth_kind: SessionAuthKind
  has_password: boolean
  has_key_passphrase: boolean
  has_key: boolean
  store_secret: boolean
}

/** create omits `id`; on update, empty/omitted secrets keep their stored value */
export interface SessionInput {
  id?: string
  name: string
  host: string
  port: number
  username: string
  auth_kind: SessionAuthKind
  password?: string | null
  key_path?: string | null
  key_content?: string | null
  key_passphrase?: string | null
  store_secret: boolean
}

class ApiError extends Error {
  status: number
  constructor(status: number, msg: string) {
    super(msg)
    this.status = status
  }
}
export { ApiError }

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {}
  const token = getToken()
  if (token) headers['x-sshero-token'] = token
  if (body !== undefined) headers['Content-Type'] = 'application/json'

  const res = await fetch(url, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`
    try {
      const j = await res.json()
      if (j.error) msg = j.error
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(res.status, msg)
  }
  if (res.status === 204) return undefined as T
  return (await res.json()) as T
}

export const api = {
  health: () => request<{ ok: boolean }>('GET', '/api/health'),
  listConnections: () =>
    request<{ connections: ConnectionInfo[] }>('GET', '/api/connections'),
  connect: (p: ConnectParams) => request<ConnectionInfo>('POST', '/api/connections', p),
  close: (id: string) => request<unknown>('DELETE', `/api/connections/${id}`),
  reconnect: (id: string) =>
    request<ConnectionInfo>('POST', `/api/connections/${id}/reconnect`),
  list: (id: string, path: string) =>
    request<ListResponse>('GET', `/api/connections/${id}/list?path=${encodeURIComponent(path)}`),
  mkdir: (id: string, path: string) =>
    request<unknown>('POST', `/api/connections/${id}/mkdir`, { path }),
  remove: (id: string, path: string, recursive: boolean) =>
    request<unknown>('POST', `/api/connections/${id}/remove`, { path, recursive }),
  rename: (id: string, from: string, to: string) =>
    request<unknown>('POST', `/api/connections/${id}/rename`, { from, to }),

  // saved session profiles
  listSessions: () => request<SessionMeta[]>('GET', '/api/sessions'),
  createSession: (s: SessionInput) => request<SessionMeta>('POST', '/api/sessions', s),
  updateSession: (id: string, s: SessionInput) =>
    request<SessionMeta>('PUT', `/api/sessions/${id}`, s),
  deleteSession: (id: string) => request<unknown>('DELETE', `/api/sessions/${id}`),
  connectSession: (id: string, secret?: string) =>
    request<ConnectionInfo>('POST', `/api/sessions/${id}/connect`, { secret: secret || null }),

  /** desktop shell: upload a local file/dir (absolute path) into remote dir */
  uploadPath: (connId: string, dir: string, localPath: string) =>
    request<{ files: number; written: number }>('POST', `/api/connections/${connId}/upload-path`, {
      dir,
      local_path: localPath,
    }),
  /** desktop shell: stream a remote file/dir into an absolute local path */
  downloadToLocal: (connId: string, path: string, dest: string) =>
    request<{ written: number }>('POST', `/api/connections/${connId}/download-to-local`, {
      path,
      dest,
    }),

  /** fetch a binary resource (with token header) and trigger a browser download */
  async download(connId: string, path: string, asDir: boolean): Promise<void> {
    const route = asDir ? 'download-dir' : 'download'
    const url = `/api/connections/${connId}/${route}?path=${encodeURIComponent(path)}`
    const headers: Record<string, string> = {}
    const token = getToken()
    if (token) headers['x-sshero-token'] = token
    const res = await fetch(url, { headers })
    if (!res.ok) {
      let msg = `${res.status} ${res.statusText}`
      try {
        const j = await res.json()
        if (j.error) msg = j.error
      } catch {
        /* ignore */
      }
      throw new ApiError(res.status, msg)
    }
    const blob = await res.blob()
    const cd = res.headers.get('Content-Disposition') ?? ''
    const m = /filename\*=UTF-8''([^;]+)/.exec(cd) || /filename="([^"]+)"/.exec(cd)
    const name = m ? decodeURIComponent(m[1]) : asDir ? 'download.tar.gz' : 'download'
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = name
    a.click()
    setTimeout(() => URL.revokeObjectURL(a.href), 5000)
  },

  /** stream one local File up; rel may contain subdirectories */
  upload(connId: string, dir: string, rel: string, file: File): Promise<{ written: number }> {
    const url =
      `/api/connections/${connId}/upload?dir=${encodeURIComponent(dir)}` +
      `&rel=${encodeURIComponent(rel)}`
    const headers: Record<string, string> = {}
    const token = getToken()
    if (token) headers['x-sshero-token'] = token
    return fetch(url, { method: 'POST', headers, body: file })
      .then(async (res) => {
        if (!res.ok) {
          let msg = `${res.status} ${res.statusText}`
          try {
            const j = await res.json()
            if (j.error) msg = j.error
          } catch {
            /* ignore */
          }
          throw new ApiError(res.status, msg)
        }
        return (await res.json()) as { written: number }
      })
  },
}

export function fmtSize(n: number): string {
  if (n >= 1 << 30) return (n / (1 << 30)).toFixed(2) + ' GiB'
  if (n >= 1 << 20) return (n / (1 << 20)).toFixed(2) + ' MiB'
  if (n >= 1 << 10) return (n / (1 << 10)).toFixed(1) + ' KiB'
  return n + ' B'
}

export function fmtMode(mode: number): string {
  // low 12 bits of the SFTP mode field: suid|sgid|sticky|rwx*3
  const bits = mode & 0o7777
  const chars = ['---', '--x', '-w-', '-wx', 'r--', 'r-x', 'rw-', 'rwx']
  const g = (shift: number) => chars[(bits >> shift) & 0o7]
  let special = ''
  if (bits & 0o4000) special += 's'
  if (bits & 0o2000) special += 's'
  if (bits & 0o1000) special += 't'
  return (special || '-') + g(6) + g(3) + g(0)
}

export function fmtTime(secs: number): string {
  if (!secs) return '-'
  return new Date(secs * 1000).toLocaleString()
}
