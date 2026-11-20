<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import {
  api,
  fmtMode,
  fmtSize,
  fmtTime,
  type RemoteEntry,
} from '../api'
import { isTauri, saveAndDownload, electronBridge, electronPathsForFiles } from '../bridge'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'
import ComboDialog from './ComboDialog.vue'
import PropertiesDialog from './PropertiesDialog.vue'

const props = defineProps<{ connId: string; active: boolean; initialPath: string }>()
const emit = defineEmits<{
  notify: [msg: string, kind?: 'info' | 'err' | 'ok']
  'update:cwd': [path: string]
}>()

// ---------- browsing ----------
const cwd = ref('')
const entries = ref<RemoteEntry[]>([])
const loading = ref(false)
const selected = reactive(new Set<string>())

const breadcrumbs = computed(() => {
  const parts = cwd.value.split('/').filter(Boolean)
  const out: { label: string; path: string }[] = [{ label: '/', path: '/' }]
  let acc = ''
  for (const p of parts) {
    acc += '/' + p
    out.push({ label: p, path: acc })
  }
  return out
})

async function enterPath(path: string) {
  if (!props.connId) {
    cwd.value = ''
    entries.value = []
    return
  }
  loading.value = true
  selected.clear()
  try {
    const r = await api.list(props.connId, path)
    cwd.value = r.path
    entries.value = r.entries
    emit('update:cwd', r.path)
  } catch (e) {
    emit('notify', (e as Error).message, 'err')
    entries.value = []
  } finally {
    loading.value = false
  }
}

watch(
  () => props.connId,
  (id) => (id ? enterPath(props.initialPath) : enterPath('')),
)

function openEntry(e: RemoteEntry) {
  if (e.is_dir && !e.is_symlink) enterPath(e.path)
  else downloadOne(e)
}

function goUp() {
  if (cwd.value === '/' || cwd.value === '') return
  const idx = cwd.value.lastIndexOf('/')
  enterPath(idx <= 0 ? '/' : cwd.value.slice(0, idx))
}

// ---------- selection ----------
function clickRow(e: RemoteEntry, ev: MouseEvent) {
  if (justDragged > Date.now() - 300) return // click fired right after a drag
  if (ev.ctrlKey || ev.metaKey) {
    selected.has(e.path) ? selected.delete(e.path) : selected.add(e.path)
  } else {
    selected.clear()
    selected.add(e.path)
  }
}
const selectedEntries = computed(() => entries.value.filter((e) => selected.has(e.path)))
const activeEntry = computed<RemoteEntry | null>(() => {
  if (ctxTarget.value) return entries.value.find((e) => e.path === ctxTarget.value) ?? null
  return selectedEntries.value.length === 1 ? selectedEntries.value[0] : null
})

// ---------- context menu ----------
const menu = reactive({ shown: false, x: 0, y: 0 })
const ctxTarget = ref<string | null>(null)
const menuBlank = ref(false)

function rowMenu(ev: MouseEvent, e: RemoteEntry | null) {
  ev.preventDefault()
  if (e) {
    if (!selected.has(e.path)) {
      selected.clear()
      selected.add(e.path)
    }
    ctxTarget.value = e.path
    menuBlank.value = false
  } else {
    ctxTarget.value = null
    menuBlank.value = true
  }
  menu.x = ev.clientX
  menu.y = ev.clientY
  menu.shown = true
}

const menuItems = computed<MenuItem[]>(() => {
  const t = menuBlank.value ? null : activeEntry.value
  if (!t) {
    return [
      { label: '新建文件夹…', action: 'mkdir', disabled: !props.connId },
      { label: '上传文件…', action: 'up-files', disabled: !props.connId },
      { label: '上传文件夹…', action: 'up-dir', disabled: !props.connId },
      { label: '刷新', action: 'refresh' },
    ]
  }
  const multi = selected.size > 1
  return [
    { label: multi ? `下载 ${selected.size} 项` : t.is_dir && !t.is_symlink ? '打开' : '下载', action: 'download' },
    { label: '重命名…', action: 'rename', disabled: multi },
    { label: '属性', action: 'props', disabled: multi },
    { label: '删除', action: 'delete', danger: true },
    { label: '刷新', action: 'refresh' },
  ]
})

function onMenuFire(action: string) {
  menu.shown = false
  switch (action) {
    case 'download':
      downloadSelected()
      break
    case 'rename':
      renameDlg.shown = true
      break
    case 'props':
      if (activeEntry.value) propsEntry.value = activeEntry.value
      break
    case 'delete':
      delDlg.shown = true
      break
    case 'mkdir':
      mkdirDlg.shown = true
      break
    case 'up-files':
      filePick.value?.click()
      break
    case 'up-dir':
      dirPick.value?.click()
      break
    case 'refresh':
      enterPath(cwd.value)
      break
  }
}

// ---------- dialogs ----------
const propsEntry = ref<RemoteEntry | null>(null)
const renameDlg = reactive({ shown: false, name: '' })
watch(
  () => renameDlg.shown,
  (s) => {
    if (s && activeEntry.value) renameDlg.name = activeEntry.value.name
  },
)
const mkdirDlg = reactive({ shown: false, name: 'new-dir' })
const delDlg = reactive({ shown: false })

async function doRename() {
  renameDlg.shown = false
  const e = activeEntry.value
  if (!e || !props.connId) return
  const to = renameDlg.name.trim()
  if (!to || to === e.name) return
  const target = to.startsWith('/') ? to : `${cwd.value}/${to}`.replace(/\/+/g, '/')
  try {
    await api.rename(props.connId, e.path, target)
    await enterPath(cwd.value)
  } catch (err) {
    emit('notify', (err as Error).message, 'err')
  }
}

async function doMkdir() {
  mkdirDlg.shown = false
  const name = mkdirDlg.name.trim()
  if (!name || !props.connId) return
  try {
    await api.mkdir(props.connId, `${cwd.value}/${name}`.replace(/\/+/g, '/'))
    await enterPath(cwd.value)
  } catch (e) {
    emit('notify', (e as Error).message, 'err')
  }
}

async function doDelete() {
  delDlg.shown = false
  const sel = selectedEntries.value
  if (!sel.length || !props.connId) return
  for (const e of sel) {
    try {
      await api.remove(props.connId, e.path, true)
    } catch (err) {
      emit('notify', `删除 ${e.name} 失败: ${(err as Error).message}`, 'err')
    }
  }
  await enterPath(cwd.value)
}

const deleteText = computed(() => {
  const sel = selectedEntries.value
  const recursive = sel.some((e) => e.is_dir && !e.is_symlink)
  return `确认删除 ${sel.length} 项？${recursive ? '目录内所有内容将被递归删除，不可恢复。' : ''}`
})

// ---------- downloads ----------
let justDragged = 0

// in-app move: paths currently being dragged (rows), for breadcrumb drop targets
const dragSources = ref<string[]>([])
const dropTarget = ref('')

// download busy indicator (bytes % is not available from the current API, so
// this is an indeterminate "working on <name>" bar rather than a real percent)
const dl = reactive({ active: false, current: '', done: 0, total: 0 })

function parentOf(p: string): string {
  const t = p.replace(/\/+$/, '')
  const i = t.lastIndexOf('/')
  return i <= 0 ? '/' : t.slice(0, i)
}
function baseName(p: string): string {
  return p.replace(/\/+$/, '').split('/').pop() || p
}
function remoteJoin(dir: string, name: string): string {
  return dir === '/' ? `/${name}` : `${dir.replace(/\/+$/, '')}/${name}`
}

async function moveInto(target: string) {
  const srcs = dragSources.value.filter(Boolean)
  dragSources.value = []
  if (!srcs.length || !props.connId) return
  let moved = 0
  for (const src of srcs) {
    if (parentOf(src) === target) continue // already there
    try {
      await api.rename(props.connId, src, remoteJoin(target, baseName(src)))
      moved++
    } catch (e) {
      emit('notify', `移动 ${baseName(src)} 失败: ${(e as Error).message}`, 'err')
    }
  }
  if (moved) {
    emit('notify', `已移动 ${moved} 项到 ${target}`, 'ok')
    await enterPath(cwd.value)
  }
}

async function downloadOne(e: RemoteEntry) {
  if (!props.connId) return
  const asDir = e.is_dir && !e.is_symlink
  dl.active = true
  dl.current = e.name
  try {
    if (isTauri() || electronBridge()) {
      // desktop: native save dialog → backend writes the local file directly
      const suggested = asDir ? `${e.name}.tar.gz` : e.name
      const r = await saveAndDownload(suggested, (dest) =>
        api.downloadToLocal(props.connId, e.path, dest),
      )
      if (r === 'unsupported') {
        await api.download(props.connId, e.path, asDir)
      } else if (r !== 'canceled') {
        emit('notify', `已保存到 ${r.savedTo}`, 'ok')
      }
    } else {
      await api.download(props.connId, e.path, asDir)
    }
  } catch (err) {
    emit('notify', (err as Error).message, 'err')
  } finally {
    dl.active = false
    dl.current = ''
  }
}

async function downloadSelected() {
  const list = activeEntry.value
    ? selectedEntries.value.length
      ? selectedEntries.value
      : [activeEntry.value]
    : []
  dl.total = list.length
  dl.done = 0
  for (const e of list) {
    await downloadOne(e)
    dl.done++
  }
  dl.total = 0
}

// drag a row onto the app window = "take it out" gesture (browser download)
let handledDrop = false

function onRowDragStart(e: RemoteEntry, ev: DragEvent) {
  ctxTarget.value = null
  if (!selected.has(e.path)) {
    selected.clear()
    selected.add(e.path)
  }
  dragSources.value = Array.from(selected)
  ev.dataTransfer?.setData('application/x-sshero-path', e.path)
  justDragged = Date.now() + 600
}

async function onWindowDrop(ev: DragEvent) {
  if (handledDrop) {
    handledDrop = false
    return // the table area already consumed it as an upload
  }
  const raw = ev.dataTransfer?.getData('application/x-sshero-path')
  if (!raw) return // foreign drag (handled by native/entry path below)
  ev.preventDefault()
  justDragged = Date.now() + 600
  if (isTauri()) {
    emit('notify', '拖出到系统文件夹暂不支持 —— 请用右键「下载」或「保存到…」', 'info')
    return
  }
  const e = entries.value.find((x) => x.path === raw)
  if (e) await downloadOne(e)
}

// ---------- uploads ----------
const filePick = ref<HTMLInputElement | null>(null)
const dirPick = ref<HTMLInputElement | null>(null)
const up = reactive({ active: false, done: 0, total: 0, current: '' })

async function uploadFiles(files: { rel: string; file: File }[]) {
  if (!props.connId || !files.length) return
  up.active = true
  up.total = files.length
  up.done = 0
  let failed = 0
  for (const f of files) {
    up.current = f.rel
    try {
      await api.upload(props.connId, cwd.value, f.rel, f.file)
    } catch (e) {
      failed++
      emit('notify', `上传 ${f.rel} 失败: ${(e as Error).message}`, 'err')
    }
    up.done++
  }
  up.active = false
  emit('notify', failed ? `上传完成，${failed} 个失败` : `上传完成（${files.length} 个文件）`, failed ? 'err' : 'ok')
  await enterPath(cwd.value)
}

async function walkEntry(item: FileSystemEntry, prefix: string, out: { rel: string; file: File }[]) {
  if (item.isFile) {
    const f = await new Promise<File>((res, rej) => (item as FileSystemFileEntry).file(res, rej))
    out.push({ rel: prefix + item.name, file: f })
  } else if (item.isDirectory) {
    const reader = (item as FileSystemDirectoryEntry).createReader()
    for (;;) {
      const batch = await new Promise<FileSystemEntry[]>((res, rej) => reader.readEntries(res, rej))
      if (!batch.length) break
      for (const b of batch) await walkEntry(b, prefix + item.name + '/', out)
    }
  }
}

const dropActive = ref(false)

async function onDrop(ev: DragEvent) {
  ev.preventDefault()
  dropActive.value = false
  if (ev.dataTransfer?.getData('application/x-sshero-path')) {
    handledDrop = true // row dragged onto the table itself: let it bubble to the download gesture
    return
  }
  if (!props.connId) return
  // Electron: dropped File entries have real OS paths via webUtils, so route
  // everything through /upload-path (backend recurses, no webview copy).
  const droppedFiles = ev.dataTransfer?.files
  if (droppedFiles && droppedFiles.length && electronBridge()) {
    const paths = electronPathsForFiles(Array.from(droppedFiles))
    if (paths && paths.length) {
      uploadNativePaths(paths)
      return
    }
  }
  const items = ev.dataTransfer?.items
  const out: { rel: string; file: File }[] = []
  if (items && items.length && (items[0].webkitGetAsEntry?.() ?? null)) {
    for (const it of Array.from(items)) {
      if (it.kind !== 'file') continue
      const entry = it.webkitGetAsEntry()
      if (entry) await walkEntry(entry, '', out)
    }
  } else if (ev.dataTransfer?.files) {
    for (const f of Array.from(ev.dataTransfer.files)) out.push({ rel: f.name, file: f })
  }
  if (out.length) uploadFiles(out)
}

const upNative = reactive({ active: false, done: 0, total: 0, current: '' })
async function uploadNativePaths(paths: string[]) {
  if (!props.connId) return
  upNative.active = true
  upNative.total = paths.length
  upNative.done = 0
  let failed = 0
  for (const p of paths) {
    upNative.current = p
    try {
      await api.uploadPath(props.connId, cwd.value, p)
    } catch (e) {
      failed++
      emit('notify', `上传 ${p} 失败: ${(e as Error).message}`, 'err')
    }
    upNative.done++
  }
  upNative.active = false
  emit('notify', failed ? `上传完成，${failed} 项失败` : `上传完成（${paths.length} 项）`, failed ? 'err' : 'ok')
  await enterPath(cwd.value)
}

function onFilePick(ev: Event) {
  const input = ev.target as HTMLInputElement
  if (input.files) {
    uploadFiles(Array.from(input.files).map((f) => ({ rel: f.name, file: f })))
  }
  input.value = ''
}
async function onDirPick(ev: Event) {
  const input = ev.target as HTMLInputElement
  const list = input.files
  if (list) {
    const out: { rel: string; file: File }[] = []
    for (const f of Array.from(list)) {
      const rel = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name
      out.push({ rel, file: f })
    }
    await uploadFiles(out)
  }
  input.value = ''
}

// native (Tauri) drops are handled by the App shell via /upload-path;
// expose a refresh hook so it can update the list afterwards
defineExpose({ refresh: () => enterPath(cwd.value), dir: () => cwd.value })

// boot when mounted with a connection already active
if (props.connId) enterPath(props.initialPath)
</script>

<template>
  <div class="files" @dragover.prevent @drop.prevent="onWindowDrop">
    <!-- toolbar -->
    <div class="toolbar">
      <button :disabled="!connId || cwd === '/'" @click="goUp">⬆ 上级</button>
      <button :disabled="!connId" @click="enterPath(cwd)">⟳ 刷新</button>
      <span class="sep"></span>
      <button :disabled="!connId" @click="filePick?.click()">上传文件</button>
      <button :disabled="!connId" @click="dirPick?.click()">上传文件夹</button>
      <span class="sep"></span>
      <button :disabled="!selected.size" @click="downloadSelected">下载所选</button>
      <button :disabled="!connId" @click="mkdirDlg.shown = true">新建文件夹</button>
      <button :disabled="selected.size !== 1" @click="renameDlg.shown = true">重命名</button>
      <button class="danger" :disabled="!selected.size" @click="delDlg.shown = true">删除所选</button>
      <input ref="filePick" type="file" multiple hidden @change="onFilePick" />
      <input ref="dirPick" type="file" webkitdirectory hidden @change="onDirPick" />
    </div>

    <!-- breadcrumb -->
    <nav class="crumbs">
      <template v-for="(b, i) in breadcrumbs" :key="b.path">
        <span v-if="i > 0" class="muted">/</span>
        <a
          :class="{ last: i === breadcrumbs.length - 1, movetarget: dropTarget === b.path }"
          @click.prevent="enterPath(b.path)"
          @dragover.prevent="dropTarget = b.path"
          @dragleave="dropTarget === b.path ? (dropTarget = '') : null"
          @drop.prevent="dropTarget = ''; moveInto(b.path)"
        >
          {{ b.label }}
        </a>
      </template>
      <span class="muted tip">拖入=上传 · 拖条目到面包屑=移动 · 拖到窗口=下载 · 右键=菜单</span>
    </nav>

    <div v-if="up.active" class="progress">
      上传中 {{ up.done }}/{{ up.total }} —— {{ up.current }}
      <progress :value="up.done" :max="up.total"></progress>
    </div>
    <div v-if="upNative.active" class="progress">
      本地拖入上传 {{ upNative.done }}/{{ upNative.total }} —— {{ upNative.current }}
      <progress :value="upNative.done" :max="upNative.total"></progress>
    </div>
    <div v-if="dl.active" class="progress">
      <template v-if="dl.total > 1">下载中 {{ dl.done }}/{{ dl.total }} —— </template>
      <template v-else>下载中 —— </template>{{ dl.current }}
      <progress class="indet" indeterminate></progress>
    </div>

    <!-- table -->
    <div
      class="table-wrap"
      :class="{ drophint: dropActive }"
      @dragenter.prevent="dropActive = !!connId"
      @dragleave.self="dropActive = false"
      @drop="onDrop"
    >
      <div v-if="loading" class="hint">加载中…</div>
      <div v-else-if="!connId" class="hint">请先在左侧选择会话并连接。</div>
      <div v-else-if="!entries.length" class="hint">目录为空。</div>
      <table v-else>
        <thead>
          <tr>
            <th class="name">名称</th>
            <th>大小</th>
            <th>权限</th>
            <th>修改时间</th>
          </tr>
        </thead>
        <tbody @contextmenu="rowMenu($event, null)">
          <tr
            v-for="e in entries"
            :key="e.path"
            :class="{ sel: selected.has(e.path) }"
            draggable="true"
            @click="clickRow(e, $event)"
            @dblclick="openEntry(e)"
            @contextmenu.stop="rowMenu($event, e)"
            @dragstart="onRowDragStart(e, $event)"
          >
            <td class="name">
              <span class="icon">{{ e.is_dir && !e.is_symlink ? '📁' : e.is_symlink ? '🔗' : '📄' }}</span>
              {{ e.name }}
            </td>
            <td>{{ e.is_dir && !e.is_symlink ? '-' : fmtSize(e.size) }}</td>
            <td class="mono">{{ fmtMode(e.mode) }}</td>
            <td>{{ fmtTime(e.mtime) }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <ContextMenu
      v-if="menu.shown"
      :x="menu.x"
      :y="menu.y"
      :items="menuItems"
      @close="menu.shown = false"
      @fire="onMenuFire"
    />
    <PropertiesDialog :entry="propsEntry" @close="propsEntry = null" />
    <ComboDialog
      :shown="renameDlg.shown"
      title="重命名"
      label="新的名称（或相对路径）"
      :initial="renameDlg.name"
      confirmText="重命名"
      @ok="(v) => { renameDlg.name = v; doRename() }"
      @cancel="renameDlg.shown = false"
    />
    <ComboDialog
      :shown="mkdirDlg.shown"
      title="新建文件夹"
      label="目录名（可含相对路径 a/b）"
      :initial="mkdirDlg.name"
      confirmText="创建"
      @ok="(v) => { mkdirDlg.name = v; doMkdir() }"
      @cancel="mkdirDlg.shown = false"
    />
    <div v-if="delDlg.shown" class="modal" @click.self="delDlg.shown = false">
      <div class="panel small">
        <h2>删除确认</h2>
        <p>{{ deleteText }}</p>
        <div class="actions">
          <button @click="delDlg.shown = false">取消</button>
          <button class="danger" @click="doDelete">删除</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.files {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}
.table-wrap { flex: 1; overflow-y: auto; position: relative; }
.table-wrap.drophint::after {
  content: '松开即上传到当前目录';
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(47, 111, 208, 0.12);
  border: 2px dashed #2f6fd0;
  color: #cfe0ff;
  font-size: 15px;
  pointer-events: none;
}
.crumbs .tip { margin-left: auto; font-size: 11px; }
.crumbs a.movetarget {
  background: #2f6fd0;
  color: #fff;
  border-radius: 3px;
  padding: 1px 5px;
}
progress.indet {
  width: 120px;
  height: 8px;
  accent-color: #4aa3ff;
}
</style>
