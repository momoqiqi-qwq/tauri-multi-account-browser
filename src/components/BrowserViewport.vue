<script setup lang="ts">
import { ref } from 'vue'
import type { BrowserBounds } from '../types'

defineProps<{ empty: boolean }>()
const emit = defineEmits<{ create: [] }>()
const host = ref<HTMLElement | null>(null)

function getBounds(): BrowserBounds | null {
  const el = host.value
  if (!el) return null
  const rect = el.getBoundingClientRect()
  return { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
}

defineExpose({
  getBounds,
  element: () => host.value,
})
</script>

<template>
  <section ref="host" class="browser-viewport">
    <div v-if="empty" class="empty-state">
      <h2>打开一个账号开始使用</h2>
      <p>可以从左侧选择已有账号，或直接新建账号。每个标签页都有独立的浏览器会话。</p>
      <el-button type="primary" @click="emit('create')">新建账号</el-button>
    </div>
  </section>
</template>
