<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import {
  api,
  getToken,
  setToken,
  type ConnectionInfo,
  type SessionInput,
  type SessionMeta,
} from './api'
import { initToken, isDesktop, onDragDrop } from './bridge'
import Sidebar from './components/Sidebar.vue'
import TerminalTab from './components/TerminalTab.vue'
import FilesTab from './components/FilesTab.vue'
import SessionDialog from './components/SessionDialog.vue'
import ComboDialog from './components/ComboDialog.vue'
import SettingsDialog from './components/SettingsDialog.vue'

// ---------- toasts ----------
const toast = reactive({ msg: '', kind: 'info' as 'info' | 'err' | 'ok', shown: false })
let toastTimer = 0
function notify(msg: string, kind: 'info' | 'err' | 'ok' = 'info') {
  toast.msg = msg
  toast.kind = kind
  toast.shown = true
  clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => (toast.shown = false), kind === 'err' ? 8000 : 3000)
}
function errMsg(e: unknown): string {
  return (e as Error).message ?? String(e)
}

// ---------- sessions + live connections ----------
const sessions = ref<SessionMeta[]>([])
const connections = ref<ConnectionInfo[]>([])
const activeId = ref('')
const busyId = ref('')
const activeConn = computed(() => connections.value.find((c) => c.id === activeId.value) ?? null)

async function refreshSessions() {
  try {
    sessions.value = await api.listSessions()
  } catch (e) {
    notify(errMsg(e), 'err')
  }
}
async function refreshConnections() {
  try {
    const r = await api.listConnections()
    connections.value = r.connections
    if (!connections.value.some((c) => c.id === activeId.value)) {
      activeId.value = connections.value[0]?.id ?? ''
    }
  } catch (e) {
    notify(errMsg(e), 'err')
  }
}

async function connectSession(s: SessionMeta, secret?: string) {
  busyId.value = s.id
  try {
    const info = await api.connectSession(s.id, secret)
    const i = connections.value.findIndex((c) => c.id === info.id)
    i >= 0 ? (connections.value[i] = info) : connections.value.push(info)
    activeId.value = info.id
    notify(`已连接 ${info.username}@${info.host}:${info.port}`, 'ok')
  } catch (e) {
    notify(errMsg(e), 'err')
  } finally {
    busyId.value = ''
  }
}

async function closeConn(id: string) {
  try {
    await api.close(id)
  } catch (e) {
    notify(errMsg(e), 'err')
  }
  await refreshConnections()
}

// ---------- session editor ----------
const sessDlg = reactive({ shown: false, edit: null as SessionMeta | null })
async function saveSession(input: SessionInput) {
  try {
    const meta = input.id
      ? await api.updateSession(input.id, input)
      : await api.createSession(input)
    const i = sessions.value.findIndex((x) => x.id === meta.id)
    i >= 0 ? (sessions.value[i] = meta) : sessions.value.push(meta)
    sessDlg.shown = false
    notify(`会话「${meta.name}」已保存`, 'ok')
  } catch (e) {
    notify(errMsg(e), 'err')
  }
}
async function deleteSession(s: SessionMeta) {
  if (!confirm(`删除会话「${s.name}」？（不影响活动连接）`)) return
  try {
    await api.deleteSession(s.id)
    sessions.value = sessions.value.filter((x) => x.id !== s.id)
  } catch (e) {
    notify(errMsg(e), 'err')
  }
}

// ---------- tabs ----------
const tab = ref<'term' | 'files'>('term')
const filesCwd = ref('')
const termRef = ref<InstanceType<typeof TerminalTab> | null>(null)
const filesRef = ref<InstanceType<typeof FilesTab> | null>(null)

// ---------- token (browser mode; in Tauri the URL carries it) ----------
const tokenDlg = reactive({ shown: false, val: '' })
const settingsDlg = reactive({ shown: false })
function applyToken(v: string) {
  setToken(v.trim())
  tokenDlg.shown = false
  notify('Token 已应用', 'ok')
  void refreshConnections()
}

// ---------- backend health ----------
const backendOk = ref<boolean | null>(null)
async function checkHealth() {
  const wasOk = backendOk.value
  try {
    await api.health()
    backendOk.value = true
  } catch {
    backendOk.value = false
  }
  if (wasOk === true && backendOk.value === false) notify('后端失联', 'err')
  else if (wasOk === false && backendOk.value === true) notify('后端已恢复', 'ok')
}

// ---------- native (Tauri) file drops → /upload-path ----------
function onNativeDrop(paths: string[]) {
  if (tab.value !== 'files' || !activeId.value) {
    notify('原生文件拖入仅在「文件浏览器」Tab 且已连接时可用', 'err')
    return
  }
  const dir = filesRef.value?.dir() || '/'
  notify(`拖入 ${paths.length} 项，上传到 ${dir} …`)
  api
    .uploadPath(activeId.value, dir, paths[0])
    .then(async (r) => {
      notify(`上传完成：${r.files} 个文件 / ${r.written} 字节`, 'ok')
      await filesRef.value?.refresh()
    })
    .catch((e) => notify(errMsg(e), 'err'))
}

let unDrag: (() => void) | undefined
let healthTimer = 0

onMounted(async () => {
  initToken()
  await checkHealth()
  healthTimer = window.setInterval(checkHealth, 30000)
  if (backendOk.value === false) {
    notify('无法访问后端 /api —— 请先启动 sshero-backend', 'err')
    return
  }
  await Promise.all([refreshSessions(), refreshConnections()])
  if (activeId.value) filesCwd.value = ''
  unDrag = onDragDrop((p) => onNativeDrop(p.paths))
})
onUnmounted(() => {
  clearInterval(healthTimer)
  unDrag?.()
})
</script>

<template>
  <div class="app-root">
    <header class="topbar">
      <span class="logo">🛡</span>
      <strong>SSHero</strong>
      <span class="who mono">
        <template v-if="activeConn">{{ activeConn.username }}@{{ activeConn.host }}:{{ activeConn.port }}</template>
        <template v-else class="muted">未连接</template>
      </span>
      <span class="spacer"></span>
      <button v-if="isDesktop()" class="ghost" title="设置（数据目录等）" @click="settingsDlg.shown = true">
        ⚙ 设置
      </button>
      <button v-if="!isDesktop()" class="ghost" title="设置 API Token" @click="tokenDlg.val = getToken(); tokenDlg.shown = true">
        🔑 Token
      </button>
      <span class="dot" :class="{ ok: backendOk === true, bad: backendOk === false }" :title="backendOk ? '后端在线' : '后端失联'"></span>
    </header>

    <div class="main">
      <Sidebar
        :sessions="sessions"
        :connections="connections"
        :active-id="activeId"
        :busy-id="busyId"
        @select="activeId = $event"
        @connect="connectSession"
        @close-conn="closeConn"
        @add="sessDlg.edit = null; sessDlg.shown = true"
        @edit="(s) => { sessDlg.edit = s; sessDlg.shown = true }"
        @delete="deleteSession"
        @save="saveSession"
      />

      <section class="work">
        <div class="tabs">
          <button :class="{ on: tab === 'term' }" @click="tab = 'term'">终端</button>
          <button :class="{ on: tab === 'files' }" @click="tab = 'files'">文件浏览器</button>
        </div>
        <div class="tab-body">
          <TerminalTab
            v-show="tab === 'term'"
            ref="termRef"
            :conn-id="activeId"
            :active="tab === 'term'"
            @notify="notify"
          />
          <FilesTab
            v-show="tab === 'files'"
            ref="filesRef"
            :conn-id="activeId"
            :active="tab === 'files'"
            :initial-path="filesCwd"
            @notify="notify"
            @update:cwd="(p: string) => (filesCwd = p)"
          />
        </div>
      </section>
    </div>

    <transition name="fade">
      <div v-if="toast.shown" class="toast" :class="toast.kind">{{ toast.msg }}</div>
    </transition>

    <SessionDialog
      :shown="sessDlg.shown"
      :session="sessDlg.edit"
      @save="saveSession"
      @cancel="sessDlg.shown = false"
    />
    <ComboDialog
      :shown="tokenDlg.shown"
      title="API Token"
      label="后端 --token 对应值（桌面模式自动生成，无需填写）"
      :initial="getToken()"
      confirmText="应用"
      @ok="applyToken"
      @cancel="tokenDlg.shown = false"
    />
    <SettingsDialog
      :shown="settingsDlg.shown"
      @close="settingsDlg.shown = false"
      @notify="(m: string, k?: 'info' | 'err' | 'ok') => notify(m, k ?? 'info')"
    />
  </div>
</template>
