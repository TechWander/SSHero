<script setup lang="ts">
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { getToken } from '../api'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'

const props = defineProps<{ connId: string; active: boolean }>()
const emit = defineEmits<{ notify: [msg: string, kind?: 'info' | 'err' | 'ok'] }>()

const host = ref<HTMLDivElement | null>(null)
const status = ref<'closed' | 'open' | 'exit'>('closed')
let term: Terminal | null = null
let fit: FitAddon | null = null
let ws: WebSocket | null = null
let ro: ResizeObserver | null = null
let disposed = false
let opened = false
let lastSize = { cols: 80, rows: 24 }

function wsUrl(id: string, cols: number, rows: number): string {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  const t = getToken()
  return (
    `${proto}://${location.host}/api/connections/${id}/terminal` +
    `?cols=${cols}&rows=${rows}` +
    (t ? `&token=${encodeURIComponent(t)}` : '')
  )
}

function sendResize() {
  if (ws && ws.readyState === WebSocket.OPEN && term) {
    ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }))
  }
}

function doFit() {
  if (!term || !fit) return
  try {
    fit.fit()
  } catch {
    /* hidden container */
  }
  if (term.cols !== lastSize.cols || term.rows !== lastSize.rows) {
    lastSize = { cols: term.cols, rows: term.rows }
    sendResize()
  }
}

function teardownWs() {
  if (ws) {
    const w = ws
    ws = null
    try {
      w.onopen = w.onmessage = w.onerror = w.onclose = null
      if (w.readyState === WebSocket.OPEN || w.readyState === WebSocket.CONNECTING) w.close()
    } catch {
      /* ignore */
    }
  }
}

function ensureOpened() {
  if (!props.active || status.value === 'open' || disposed || !props.connId) return
  open()
}

function open() {
  if (!term || !props.connId) return
  teardownWs()
  status.value = 'open'
  let done = false
  const s = new WebSocket(wsUrl(props.connId, term.cols, term.rows))
  ws = s
  s.binaryType = 'arraybuffer'
  const decoder = new TextDecoder('utf-8')
  s.onopen = () => {
    if (done || ws !== s) return
    done = true
    if (!opened) {
      opened = true
      term!.writeln('\x1b[32m● SSH shell 已就绪\x1b[0m')
    }
    term!.focus()
    doFit()
  }
  s.onmessage = (ev: MessageEvent) => {
    if (ws !== s) return
    if (ev.data instanceof ArrayBuffer) {
      term!.write(decoder.decode(new Uint8Array(ev.data), { stream: true }))
    } else if (typeof ev.data === 'string') {
      try {
        const m = JSON.parse(ev.data as string)
        if (m.type === 'exit') {
          status.value = 'exit'
          term?.writeln(`\r\n\x1b[33m[shell 已退出，code=${m.code}] 右键可重连\x1b[0m`)
        } else if (m.type === 'error') {
          emit('notify', String(m.message), 'err')
        }
      } catch {
        /* ignore non-JSON control frames */
      }
    }
  }
  s.onerror = () => {
    if (ws !== s) return
    status.value = 'closed'
    term?.writeln('\r\n\x1b[31m[终端连接失败或已断开]\x1b[0m')
  }
  s.onclose = () => {
    if (ws === s) {
      ws = null
      if (status.value === 'open') status.value = 'closed'
    }
  }
}

function closeWsOnly() {
  teardownWs()
  status.value = 'closed'
}

function reconnect() {
  teardownWs()
  status.value = 'closed'
  term?.reset()
  opened = false
  if (props.connId) open()
  else term?.writeln('\x1b[33m[无活动连接 —— 请先在左侧选择/连接会话]\x1b[0m')
}

watch(
  () => props.connId,
  () => {
    teardownWs()
    status.value = 'closed'
    opened = false
    term?.reset()
    ensureOpened()
  },
)
watch(
  () => props.active,
  (a) => {
    if (a) {
      ensureOpened()
      setTimeout(doFit, 60)
    }
  },
)

// ---------- right-click menu ----------
const menu = ref({ shown: false, x: 0, y: 0 })
const menuItems: MenuItem[] = [
  { label: '复制', action: 'copy' },
  { label: '粘贴', action: 'paste' },
  { label: '清屏', action: 'clear' },
  { label: '重连终端', action: 'reconnect' },
]
function onContextMenu(ev: MouseEvent) {
  ev.preventDefault()
  menu.value = { shown: true, x: ev.clientX, y: ev.clientY }
}
function onMenuFire(action: string) {
  menu.value.shown = false
  switch (action) {
    case 'copy':
      if (term) {
        const sel = term.getSelection()
        if (sel) navigator.clipboard?.writeText(sel).catch(() => undefined)
      }
      break
    case 'paste':
      navigator.clipboard
        ?.readText()
        .then((t) => term?.paste(t))
        .catch(() => emit('notify', '无法读取剪贴板', 'err'))
      break
    case 'clear':
      term?.clear()
      break
    case 'reconnect':
      reconnect()
      break
  }
}

onMounted(() => {
  if (!host.value) return
  term = new Terminal({
    fontFamily: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace",
    fontSize: 13,
    cursorBlink: true,
    scrollback: 5000,
    theme: { background: '#0f1218', foreground: '#d5dae2' },
  })
  fit = new FitAddon()
  term.loadAddon(fit)
  term.attachCustomKeyEventHandler((ev) => {
    // let the browser/tauri handle copy/paste shortcuts; xterm gets the rest
    const ctrl = ev.ctrlKey || ev.metaKey
    if (ev.type === 'keydown' && ctrl && ev.key.toLowerCase() === 'c' && term?.hasSelection()) {
      const sel = term.getSelection()
      if (sel) navigator.clipboard?.writeText(sel).catch(() => undefined)
      return false
    }
    if (ev.type === 'keydown' && ctrl && ev.key.toLowerCase() === 'v') {
      navigator.clipboard?.readText().then(
        (t) => term?.paste(t),
        () => undefined,
      )
      return false
    }
    return true
  })
  term.onData((d) => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(new TextEncoder().encode(d))
    }
  })
  term.onResize(() => sendResize())
  term.open(host.value)
  doFit()
  term.writeln('\x1b[33m[提示] 左侧选择会话并连接后，此处将自动打开交互式 shell；右键菜单支持复制/粘贴/清屏/重连。\x1b[0m')
  if (props.connId) open()

  ro = new ResizeObserver(() => doFit())
  ro.observe(host.value)
})

onBeforeUnmount(() => {
  disposed = true
  ro?.disconnect()
  teardownWs()
  term?.dispose()
  term = null
})

defineExpose({ doFit, reconnect, closeWsOnly })
</script>

<template>
  <div class="term-wrap">
    <div class="term-bar">
      <span class="st" :class="status">
        {{ status === 'open' ? '● 已连接' : status === 'exit' ? '○ shell 已退出' : '○ 未连接' }}
      </span>
      <span v-if="connId" class="mono muted">connection {{ connId.slice(0, 8) }}</span>
      <span class="spacer"></span>
      <button class="ghost" :disabled="!connId" title="重连终端" @click="reconnect">⟳ 重连</button>
    </div>
    <div ref="host" class="term-host" @contextmenu="onContextMenu"></div>
    <ContextMenu
      v-if="menu.shown"
      :x="menu.x"
      :y="menu.y"
      :items="menuItems"
      @close="menu.shown = false"
      @fire="onMenuFire"
    />
  </div>
</template>

<style scoped>
.term-wrap {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: #0f1218;
}
.term-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 10px;
  font-size: 12px;
  color: #9aa4b2;
  border-bottom: 1px solid #22262e;
}
.term-bar .spacer { flex: 1; }
.st.open { color: #3fbf6f; }
.st.exit { color: #d9a441; }
.st.closed { color: #7f8ea3; }
.ghost {
  background: none;
  border: none;
  color: #9db4d0;
  cursor: pointer;
  font-size: 12px;
}
.ghost:hover { color: #fff; }
.ghost:disabled { opacity: 0.4; cursor: default; }
.term-host {
  flex: 1;
  min-height: 0;
  padding: 4px 0 0 6px;
  overflow: hidden;
}
</style>
