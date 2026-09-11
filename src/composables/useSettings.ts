import { ref, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import type { AppSettings, DownloadEntry } from '../types'

/**
 * 扩展名白名单规范化：去前导点、转小写、去重、丢弃非法项。
 * 口径与 Rust 侧 `normalize_ai_exts` 保持一致，避免前后端判定分叉。
 */
export function normalizeExtensionWhitelist(raw: string): string {
  const seen = new Set<string>()
  return raw
    .split(',')
    .map((item) => item.trim().replace(/^\.+/, '').toLowerCase())
    .filter((item) => /^[a-z0-9]{1,12}$/.test(item))
    .filter((item) => {
      if (seen.has(item)) return false
      seen.add(item)
      return true
    })
    .join(',')
}

export interface SettingsApi {
  /** 全局下载设置；null 表示尚未加载成功 */
  dlSettings: Ref<AppSettings | null>
  downloadHistory: Ref<DownloadEntry[]>
  loadSettings(): Promise<void>
  loadDownloadHistory(): Promise<void>
  /**
   * 保存设置。`onSaved` 用于让宿主重新拉取账号列表 ——
   * 继承模式账号的主页依赖 `global_default_url`，改了全局主页要立刻反映到账号上。
   */
  saveSettings(settings: AppSettings, onSaved?: () => Promise<void> | void): Promise<void>
  /** 收到 `profile:download` 事件后把新记录插到队首，并按历史上限截断 */
  pushDownloadEntry(entry: DownloadEntry): void
}

/** 全局设置与下载历史的加载/保存。不依赖其它 composable。 */
export function useSettings(): SettingsApi {
  const dlSettings = ref<AppSettings | null>(null)
  const downloadHistory = ref<DownloadEntry[]>([])

  async function loadSettings() {
    dlSettings.value = await invoke<AppSettings>('get_app_settings').catch(() => null)
  }

  async function loadDownloadHistory() {
    downloadHistory.value = await invoke<DownloadEntry[]>('get_download_history').catch(() => [])
  }

  function pushDownloadEntry(entry: DownloadEntry) {
    const limit = dlSettings.value?.download_history_limit ?? 200
    downloadHistory.value = [entry, ...downloadHistory.value].slice(0, limit)
  }

  async function saveSettings(settings: AppSettings, onSaved?: () => Promise<void> | void) {
    const normalized = { ...settings, ai_exts: normalizeExtensionWhitelist(settings.ai_exts) }
    try {
      await invoke('set_app_settings', { settings: normalized })
      // 保存后重新读一次：Rust 侧会做规范化并强制部分字段（如 skip_downloaded_files），
      // 回读可以避免前端状态与落盘结果不一致。
      dlSettings.value = await invoke<AppSettings>('get_app_settings')
      await onSaved?.()
      ElMessage.success('应用设置已保存；继承模式账号已同步全局默认主页')
    } catch (error) {
      ElMessage.error(`保存应用设置失败：${String(error)}`)
    }
  }

  return {
    dlSettings,
    downloadHistory,
    loadSettings,
    loadDownloadHistory,
    saveSettings,
    pushDownloadEntry,
  }
}
