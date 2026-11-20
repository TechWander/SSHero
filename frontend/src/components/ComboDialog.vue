<script setup lang="ts">
import { ref, watch } from 'vue'

const props = defineProps<{
  shown: boolean
  title: string
  label?: string
  password?: boolean
  placeholder?: string
  initial?: string
  confirmText?: string
}>()
const emit = defineEmits<{ ok: [value: string]; cancel: [] }>()

const val = ref('')
const box = ref<HTMLInputElement | null>(null)
watch(
  () => props.shown,
  (s) => {
    if (s) {
      val.value = props.initial ?? ''
      setTimeout(() => box.value?.focus(), 30)
    }
  },
)
</script>

<template>
  <div v-if="shown" class="modal" @click.self="emit('cancel')">
    <div class="panel small">
      <h2>{{ title }}</h2>
      <label v-if="label" class="row-label">{{ label }}</label>
      <input
        ref="box"
        v-model="val"
        :type="password ? 'password' : 'text'"
        :placeholder="placeholder ?? ''"
        @keyup.enter="val.trim() && emit('ok', val.trim())"
        @keyup.esc="emit('cancel')"
      />
      <div class="actions">
        <button @click="emit('cancel')">取消</button>
        <button class="primary" :disabled="!val.trim()" @click="emit('ok', val.trim())">
          {{ confirmText ?? '确定' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.row-label {
  display: block;
  margin: 6px 0 4px;
  color: #9aa4b2;
  font-size: 13px;
}
.panel.small {
  width: 380px;
}
.panel.small input {
  width: 100%;
  box-sizing: border-box;
}
</style>
