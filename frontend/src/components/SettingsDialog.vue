<script setup lang="ts">
import { ref, watch } from 'vue'
import { getSettings, chooseDataDir, isElectron, type ShellSettings } from '../bridge'

const props = defineProps<{ shown: boolean }>()
const emit = defineEmits<{ close: []; notify: [msg: string, kind?: 'info' | 'err' | 'ok'] }>()

const settings = ref<ShellSettings | null>(null)
const pendingDir = ref('')

async function reload() {
  settings.value = await getSettings()
  pendingDir.value = settings.value?.dataDir ?? ''
}

watch(
  () => props.shown,
  (s) => {
    if (s) void reload()
  },
)

async function onChoose() {
  const dir = await chooseDataDir()
  if (dir) {
    pendingDir.value = dir
    emit('notify', '数据目录已记录，重启应用后生效', 'ok')
  }
}
</script>

<template>
  <div v-if="shown" class="modal" @click.self="emit('close')">
    <div class="panel sess">
      <h2>设置</h2>
      <div class="grid">
        <label>数据目录</label>
        <div class="dirrow">
          <input :value="pendingDir" readonly placeholder="（由桌面壳管理）" />
          <button v-if="isElectron()" type="button" class="ghost" @click="onChoose">更改目录…</button>
        </div>
      </div>
      <p v-if="settings" class="muted">
        会话档案与 known_hosts 保存在此目录。默认位置：
        <code>{{ settings.defaultDataDir }}</code><br />
        修改后需<strong>重启应用</strong>才会用新目录加载已有会话。
      </p>
      <p v-else class="muted">
        仅桌面版（Electron）可在应用内更改数据目录；浏览器模式由后端
        <code>--data-dir</code> 启动参数决定。
      </p>
      <div class="actions">
        <button @click="emit('close')">关闭</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.panel.sess {
  width: 560px;
}
.dirrow {
  display: flex;
  gap: 6px;
  align-items: center;
}
.dirrow input {
  flex: 1;
}
.dirrow .ghost {
  background: #22272f;
  border: 1px solid #3d4450;
  color: #cdd5df;
  border-radius: 4px;
  padding: 4px 10px;
  cursor: pointer;
  white-space: nowrap;
}
.dirrow .ghost:hover {
  background: #2b323c;
}
code {
  color: #9fd0ff;
  background: #10131a;
  padding: 1px 4px;
  border-radius: 3px;
}
</style>
