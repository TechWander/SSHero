<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import type { ConnectionInfo, SessionInput, SessionMeta } from '../api'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'
import ComboDialog from './ComboDialog.vue'

const props = defineProps<{
  sessions: SessionMeta[]
  connections: ConnectionInfo[]
  activeId: string
  busyId: string
}>()
const emit = defineEmits<{
  (e: 'select', id: string): void
  (e: 'connect', session: SessionMeta, secret?: string): void
  (e: 'close-conn', id: string): void
  (e: 'add'): void
  (e: 'edit', s: SessionMeta): void
  (e: 'delete', s: SessionMeta): void
  (e: 'save', input: SessionInput): void
}>()

function isLive(s: SessionMeta): boolean {
  return props.connections.some((c) => c.host === s.host && c.port === s.port && c.username === s.username)
}

// ---------- session context menu ----------
const menu = reactive({ shown: false, x: 0, y: 0 })
const menuSession = ref<SessionMeta | null>(null)
const menuItems = computed<MenuItem[]>(() => [
  { label: '连接', action: 'connect', disabled: !menuSession.value },
  { label: '编辑…', action: 'edit', disabled: !menuSession.value },
  { label: '删除', action: 'delete', danger: true, disabled: !menuSession.value },
])

function openMenu(ev: MouseEvent, s: SessionMeta) {
  ev.preventDefault()
  menuSession.value = s
  menu.x = ev.clientX
  menu.y = ev.clientY
  menu.shown = true
}
function onMenuFire(action: string) {
  const s = menuSession.value
  menu.shown = false
  if (!s) return
  if (action === 'connect') tryConnect(s)
  else if (action === 'edit') emit('edit', s)
  else if (action === 'delete') emit('delete', s)
}

// ---------- connect (ask for fresh secret when profile does not store one) ----------
const ask = reactive({ shown: false, session: null as SessionMeta | null, label: '' })

function needsSecret(s: SessionMeta): boolean {
  if (s.store_secret) return false
  if (s.auth_kind === 'password') return !s.has_password
  return s.auth_kind === 'key-path' && !s.has_key_passphrase
}

function tryConnect(s: SessionMeta) {
  if (needsSecret(s)) {
    ask.session = s
    ask.label =
      s.auth_kind === 'password'
        ? `${s.username}@${s.host} 的密码`
        : `${s.username}@${s.host} 的私钥口令`
    ask.shown = true
  } else {
    emit('connect', s)
  }
}
function onAskOk(val: string) {
  const s = ask.session
  ask.shown = false
  if (s) emit('connect', s, val)
}
</script>

<template>
  <aside class="sidebar">
    <section class="sb-sec live">
      <header class="sb-head">
        <span>活动连接</span>
        <em class="muted">{{ connections.length }}</em>
      </header>
      <div v-if="!connections.length" class="muted empty-tip">尚无活动连接</div>
      <ul class="conn-list">
        <li
          v-for="c in connections"
          :key="c.id"
          :class="{ on: c.id === activeId }"
          @click="emit('select', c.id)"
        >
          <span class="live-dot"></span>
          <span class="cname mono">{{ c.username }}@{{ c.host }}:{{ c.port }}</span>
          <button class="ghost" title="断开" @click.stop="emit('close-conn', c.id)">✕</button>
        </li>
      </ul>
    </section>

    <section class="sb-sec grow">
      <header class="sb-head">
        <span>已保存的会话</span>
        <span class="sb-tools">
          <button class="ghost" title="新建会话" @click="emit('add')">＋</button>
        </span>
      </header>
      <div v-if="!sessions.length" class="muted empty-tip">点击 ＋ 创建第一个会话</div>
      <ul class="sess-list">
        <li
          v-for="s in sessions"
          :key="s.id"
          :class="{ on: isLive(s) }"
          :title="`${s.username}@${s.host}:${s.port} · ${s.auth_kind}`"
          @contextmenu="openMenu($event, s)"
          @dblclick="tryConnect(s)"
        >
          <span class="icon">{{ isLive(s) ? '🟢' : s.auth_kind === 'password' ? '🔑' : '📜' }}</span>
          <span class="col">
            <b>{{ s.name }}</b>
            <small class="mono muted">{{ s.username }}@{{ s.host }}:{{ s.port }}</small>
          </span>
          <button class="ghost" :disabled="busyId === s.id" title="连接" @click.stop="tryConnect(s)">
            {{ busyId === s.id ? '…' : '▶' }}
          </button>
          <button class="ghost" title="编辑" @click.stop="emit('edit', s)">✎</button>
          <button class="ghost del" title="删除" @click.stop="emit('delete', s)">🗑</button>
        </li>
      </ul>
    </section>

    <ContextMenu
      v-if="menu.shown"
      :x="menu.x"
      :y="menu.y"
      :items="menuItems"
      @close="menu.shown = false"
      @fire="onMenuFire"
    />
    <ComboDialog
      :shown="ask.shown"
      title="输入凭据"
      :label="ask.label"
      password
      confirmText="连接"
      @ok="onAskOk"
      @cancel="ask.shown = false"
    />
  </aside>
</template>

<style scoped>
.sidebar {
  width: 280px;
  min-width: 220px;
  display: flex;
  flex-direction: column;
  background: #14171d;
  border-right: 1px solid #2b313b;
  overflow: hidden;
}
.sb-sec { padding: 8px 0 4px; }
.sb-sec.grow { flex: 1; overflow-y: auto; }
.sb-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 4px 10px;
  font-size: 12px;
  color: #8b94a4;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
.sb-tools button { font-size: 14px; }
.empty-tip { padding: 8px 12px; font-size: 12px; }
.conn-list, .sess-list { list-style: none; margin: 0; padding: 0; }
.conn-list li, .sess-list li {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  font-size: 13px;
  cursor: pointer;
  color: #c8cfd9;
}
.conn-list li:hover, .sess-list li:hover { background: #1d222b; }
.conn-list li.on, .sess-list li.on { background: #21304a; }
.live-dot { width: 8px; height: 8px; border-radius: 50%; background: #3fbf6f; flex: none; }
.sess-list li .col { display: flex; flex-direction: column; flex: 1; overflow: hidden; }
.sess-list li b { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.sess-list li small { font-size: 11px; }
.ghost {
  background: none;
  border: none;
  color: #7f8ea3;
  cursor: pointer;
  padding: 2px 4px;
  font-size: 12px;
}
.ghost:hover { color: #dfe6ef; }
.ghost.del:hover { color: #e26d6d; }
.ghost:disabled { opacity: 0.4; cursor: default; }
</style>
