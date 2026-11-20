<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import type { SessionAuthKind, SessionInput, SessionMeta } from '../api'
import { defaultKeyPaths, localFileExists, pickKeyFile, isElectron } from '../bridge'

const props = defineProps<{ shown: boolean; session: SessionMeta | null }>()
const emit = defineEmits<{ save: [input: SessionInput]; cancel: [] }>()

interface Form extends SessionInput {
  storePassword: boolean
  storePassphrase: boolean
}

function blank(): Form {
  return {
    name: '',
    host: '',
    port: 22,
    username: 'root',
    auth_kind: 'password',
    password: '',
    key_path: '',
    key_content: '',
    key_passphrase: '',
    store_secret: true,
    storePassword: true,
    storePassphrase: true,
  }
}

const f = reactive<Form>(blank())

// private-key path UX: prefilled default + existence status + native browse
const keyStatus = ref<'unknown' | 'found' | 'missing'>('unknown')
const canBrowse = isElectron()

async function refreshKeyStatus() {
  if (!f.key_path || f.key_path.startsWith('(已存储')) {
    keyStatus.value = 'unknown'
    return
  }
  keyStatus.value = (await localFileExists(f.key_path)) ? 'found' : 'missing'
}

// pick the first default that exists, else the ed25519 default as a hint
async function fillDefaultKey() {
  const cands = await defaultKeyPaths()
  if (!cands.length) return
  for (const c of cands) {
    if (await localFileExists(c)) {
      f.key_path = c
      await refreshKeyStatus()
      return
    }
  }
  if (!f.key_path) f.key_path = cands[0]
  await refreshKeyStatus()
}

async function onBrowseKey() {
  const p = await pickKeyFile()
  if (p) {
    f.key_path = p
    await refreshKeyStatus()
  }
}

watch(
  () => props.shown,
  (s) => {
    if (!s) return
    Object.assign(f, blank())
    keyStatus.value = 'unknown'
    const m = props.session
    if (m) {
      f.id = m.id
      f.name = m.name
      f.host = m.host
      f.port = m.port
      f.username = m.username
      f.auth_kind = m.auth_kind
      f.key_path = m.has_key ? '(已存储，留空则保持不变)' : ''
      f.storePassword = m.has_password
      f.storePassphrase = m.has_key_passphrase
    }
  },
)

// when the user chooses key-path auth on a fresh session, prefill a default
watch(
  () => f.auth_kind,
  (k) => {
    if (k === 'key-path' && !props.session && !f.key_path) void fillDefaultKey()
  },
)

// key_path is never echoed back; a placeholder marks "already stored" so the
// user must deliberately retype it to overwrite
function clean(v?: string | null): string | undefined {
  const s = (v ?? '').trim()
  return s && !s.startsWith('(已存储') ? s : undefined
}

function submit() {
  const input: SessionInput = {
    id: f.id,
    name: f.name.trim() || `${f.username}@${f.host}`,
    host: f.host.trim(),
    port: f.port || 22,
    username: f.username.trim(),
    auth_kind: f.auth_kind as SessionAuthKind,
    password: f.auth_kind === 'password' ? clean(f.password) : undefined,
    key_path: f.auth_kind === 'key-path' ? clean(f.key_path) : undefined,
    key_content: f.auth_kind === 'key-content' ? f.key_content || undefined : undefined,
    key_passphrase: f.auth_kind !== 'password' ? clean(f.key_passphrase) : undefined,
    store_secret:
      f.auth_kind === 'password'
        ? f.storePassword
        : f.storePassphrase || !!clean(f.key_content),
  }
  emit('save', input)
}
</script>

<template>
  <div v-if="shown" class="modal" @click.self="emit('cancel')">
    <div class="panel sess">
      <h2>{{ session ? '编辑会话' : '新建会话' }}</h2>
      <div class="grid">
        <label>名称</label><input v-model="f.name" placeholder="留空自动用 user@host" />
        <label>主机地址</label><input v-model="f.host" placeholder="192.168.1.10 或 example.com" />
        <label>端口</label><input v-model.number="f.port" type="number" min="1" max="65535" />
        <label>用户名</label><input v-model="f.username" />
        <label>认证方式</label>
        <div class="radios">
          <label><input v-model="f.auth_kind" type="radio" value="password" /> 密码</label>
          <label><input v-model="f.auth_kind" type="radio" value="key-path" /> 密钥路径</label>
          <label><input v-model="f.auth_kind" type="radio" value="key-content" /> 私钥内容</label>
        </div>

        <template v-if="f.auth_kind === 'password'">
          <label>密码</label>
          <input v-model="f.password" type="password" :placeholder="session?.has_password ? '已存储，留空则保持不变' : ''" />
          <label></label>
          <label class="chk"><input v-model="f.storePassword" type="checkbox" /> 在后端持久保存密码（取消则每次连接时询问）</label>
        </template>

        <template v-else-if="f.auth_kind === 'key-path'">
          <label>私钥路径</label>
          <div class="keyrow">
            <input
              v-model="f.key_path"
              placeholder="后端所在机器的绝对路径，如 /home/me/.ssh/id_ed25519"
              @change="refreshKeyStatus"
              @blur="refreshKeyStatus"
            />
            <button v-if="canBrowse" type="button" class="ghost" @click="onBrowseKey">浏览…</button>
          </div>
          <label></label>
          <p class="keystatus" :class="keyStatus">
            <template v-if="keyStatus === 'found'">✓ 已检测到该私钥文件</template>
            <template v-else-if="keyStatus === 'missing'">⚠ 该文件当前不存在（可手动改正或点“浏览…”选择）</template>
            <template v-else>已按当前系统预填默认私钥路径</template>
          </p>
          <label>密钥口令</label>
          <input v-model="f.key_passphrase" type="password" :placeholder="session?.has_key_passphrase ? '已存储，留空则保持不变' : ''" />
          <label></label>
          <label class="chk"><input v-model="f.storePassphrase" type="checkbox" /> 持久保存密钥口令</label>
        </template>

        <template v-else>
          <label>私钥内容</label>
          <textarea v-model="f.key_content" rows="6" spellcheck="false"
            :placeholder="session?.has_key ? '已存储（出于安全不回显）；粘贴新内容将覆盖' : '-----BEGIN OPENSSH PRIVATE KEY-----\n…'"></textarea>
          <label>密钥口令</label>
          <input v-model="f.key_passphrase" type="password" :placeholder="session?.has_key_passphrase ? '已存储，留空则保持不变' : ''" />
        </template>
      </div>
      <p class="muted">
        私密信息仅保存在后端数据目录（0600），API 只回传“是否已存储”，永不回显。首次连接采用 TOFU 记录主机密钥。
      </p>
      <div class="actions">
        <button @click="emit('cancel')">取消</button>
        <button class="primary" :disabled="!f.host || !f.username" @click="submit">保存</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.panel.sess {
  width: 560px;
}
textarea {
  width: 100%;
  box-sizing: border-box;
  background: #10131a;
  color: #d5dae2;
  border: 1px solid #3d4450;
  border-radius: 4px;
  font-family: 'Cascadia Code', Consolas, monospace;
  font-size: 12px;
  padding: 6px;
  resize: vertical;
}
.radios label {
  margin-right: 14px;
}
.chk {
  color: #9aa4b2;
  font-size: 12px;
}
.keyrow {
  display: flex;
  gap: 6px;
  align-items: center;
}
.keyrow input {
  flex: 1;
}
.keyrow .ghost {
  background: #22272f;
  border: 1px solid #3d4450;
  color: #cdd5df;
  border-radius: 4px;
  padding: 4px 10px;
  cursor: pointer;
  white-space: nowrap;
}
.keyrow .ghost:hover {
  background: #2b323c;
}
.keystatus {
  margin: 0;
  font-size: 12px;
}
.keystatus.found {
  color: #3fbf6f;
}
.keystatus.missing {
  color: #e0a34a;
}
.keystatus.unknown {
  color: #7f8ea3;
}
</style>
