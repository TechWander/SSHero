<script setup lang="ts">
import { computed } from 'vue'
import { fmtMode, fmtSize, fmtTime, type RemoteEntry } from '../api'

const props = defineProps<{ entry: RemoteEntry | null }>()
const emit = defineEmits<{ close: [] }>()

function octMode(mode: number): string {
  return (mode & 0o7777).toString(8).padStart(4, '0')
}
const kind = computed(() => {
  const e = props.entry
  if (!e) return ''
  if (e.is_symlink) return '符号链接'
  return e.is_dir ? '目录' : '文件'
})
</script>

<template>
  <div v-if="entry" class="modal" @click.self="emit('close')">
    <div class="panel props">
      <h2>属性 —— {{ entry.name }}</h2>
      <table>
        <tbody>
          <tr><td>完整路径</td><td class="mono">{{ entry.path }}</td></tr>
          <tr><td>类型</td><td>{{ kind }}</td></tr>
          <tr><td>大小</td><td>{{ entry.is_dir && !entry.is_symlink ? '-' : fmtSize(entry.size) }}<span class="muted mono"> ({{ entry.size }} 字节)</span></td></tr>
          <tr><td>权限</td><td class="mono">{{ fmtMode(entry.mode) }}  ({{ octMode(entry.mode) }})</td></tr>
          <tr><td>所有者 UID</td><td class="mono">{{ entry.uid ?? '-' }}</td></tr>
          <tr><td>所属组 GID</td><td class="mono">{{ entry.gid ?? '-' }}</td></tr>
          <tr><td>修改时间</td><td>{{ fmtTime(entry.mtime) }}</td></tr>
        </tbody>
      </table>
      <div class="actions">
        <button class="primary" @click="emit('close')">关闭</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.panel.props {
  width: 520px;
}
.panel.props table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}
.panel.props td {
  padding: 5px 8px;
  border-bottom: 1px solid #2b313b;
  vertical-align: top;
  word-break: break-all;
}
.panel.props td:first-child {
  color: #9aa4b2;
  white-space: nowrap;
  width: 110px;
}
</style>
