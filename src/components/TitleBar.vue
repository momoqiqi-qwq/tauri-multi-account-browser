<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import appIcon from '../../src-tauri/icons/32x32.png'

/**
 * 自绘标题栏。主窗口在 tauri.conf.json 里关掉了 decorations，窗口拖动、
 * 双击最大化和三个控制按钮全部由这一条承担；权限见 capabilities/default.json。
 */
const win = getCurrentWindow()
const maximized = ref(false)
let unlistenResize: (() => void) | undefined

async function syncMaximized() {
  try {
    maximized.value = await win.isMaximized()
  } catch {
    /* 拿不到最大化状态时保持当前图标，下一次 resize 会再同步 */
  }
}

function control(action: 'minimize' | 'toggleMaximize' | 'close') {
  void win[action]().catch(() => undefined)
}

onMounted(() => {
  void syncMaximized()
  win
    .onResized(syncMaximized)
    .then((unlisten) => { unlistenResize = unlisten })
    .catch(() => undefined)
})
onBeforeUnmount(() => unlistenResize?.())
</script>

<template>
  <header class="title-bar" data-tauri-drag-region="deep">
    <img class="title-bar-icon" :src="appIcon" alt="" draggable="false" />
    <span class="title-bar-name">Multi Account Browser</span>
    <span class="title-bar-spacer" />
    <div class="window-controls">
      <button class="win-btn" type="button" title="最小化" @click="control('minimize')">
        <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M1 6h10" /></svg>
      </button>
      <button
        class="win-btn"
        type="button"
        :title="maximized ? '向下还原' : '最大化'"
        @click="control('toggleMaximize')"
      >
        <svg v-if="!maximized" viewBox="0 0 12 12" aria-hidden="true">
          <rect x="1.5" y="1.5" width="9" height="9" rx="1" />
        </svg>
        <svg v-else viewBox="0 0 12 12" aria-hidden="true">
          <rect x="1.5" y="3.5" width="7" height="7" rx="1" />
          <path d="M3.5 1.5h7v7" />
        </svg>
      </button>
      <button class="win-btn win-close" type="button" title="关闭" @click="control('close')">
        <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M1.5 1.5l9 9M10.5 1.5l-9 9" /></svg>
      </button>
    </div>
  </header>
</template>
