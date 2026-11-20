<script setup lang="ts">
import { onBeforeUnmount, onMounted } from 'vue'

export interface MenuItem {
  label: string
  danger?: boolean
  disabled?: boolean
  action: string
}
const props = defineProps<{ x: number; y: number; items: MenuItem[] }>()
const emit = defineEmits<{ close: []; fire: [action: string] }>()

function clamp(v: number, size: number) {
  return Math.max(4, Math.min(v, window.innerWidth - size - 4))
}
function clampY(v: number, size: number) {
  return Math.max(4, Math.min(v, window.innerHeight - size - 4))
}

function onDocDown(e: MouseEvent) {
  if (!(e.target as HTMLElement).closest('.ctx-menu')) close()
}
function close() {
  emit('close')
}
onMounted(() => document.addEventListener('mousedown', onDocDown))
onBeforeUnmount(() => document.removeEventListener('mousedown', onDocDown))
</script>

<template>
  <div class="ctx-mask" @contextmenu.prevent="close" @click="close">
    <ul
      class="ctx-menu"
      :style="{ left: clamp(props.x, 220) + 'px', top: clampY(props.y, 260) + 'px' }"
      @click.stop
    >
      <li v-if="!items.length" class="empty">（无操作）</li>
      <li
        v-for="it in items"
        :key="it.action"
        :class="{ danger: it.danger, disabled: it.disabled }"
        @click="!it.disabled && emit('fire', it.action)"
      >
        {{ it.label }}
      </li>
    </ul>
  </div>
</template>

<style scoped>
.ctx-mask {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: transparent;
}
.ctx-menu {
  position: absolute;
  min-width: 190px;
  padding: 4px 0;
  background: #23272e;
  border: 1px solid #3d4450;
  border-radius: 6px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  list-style: none;
  margin: 0;
  font-size: 13px;
}
.ctx-menu li {
  padding: 6px 14px;
  cursor: pointer;
  color: #d5dae2;
  white-space: nowrap;
}
.ctx-menu li:hover:not(.disabled):not(.empty) {
  background: #2f6fd0;
  color: #fff;
}
.ctx-menu li.danger {
  color: #e26d6d;
}
.ctx-menu li.danger:hover {
  background: #a33;
  color: #fff;
}
.ctx-menu li.disabled {
  color: #61697a;
  cursor: default;
}
.ctx-menu li.empty {
  color: #61697a;
  cursor: default;
}
</style>
