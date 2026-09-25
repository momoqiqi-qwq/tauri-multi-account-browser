<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, toRef, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import { ArrowRight, Edit, FolderAdd, Grid, MagicStick, Plus, Search, Setting } from '@element-plus/icons-vue'
import { acquireWebviewSuppression } from '../lib/suppress'
import { ALL_SITES } from '../lib/siteDirectory'
import { useDirectoryScroll } from '../composables/useDirectoryScroll'
import { useWebsiteDirectory } from '../composables/useWebsiteDirectory'
import type { Profile } from '../types'
import '../website-directory.css'

const props = defineProps<{
  profiles: Profile[]
  globalDefaultUrl: string
  activeId: string | null
  openTabs: string[]
  loading: boolean
  error: string
  openingId: string | null
}>()
const emit = defineEmits<{
  activate: [id: string]
  create: [url?: string]
  edit: [profile: Profile]
  close: []
  retry: []
  tools: []
  generator: [url?: string]
  settings: []
}>()

const { selected, columns, query, groups, current, inSite, matches, selectSite, saveSite, removeEmptySite } =
  useWebsiteDirectory(() => props.profiles, () => props.globalDefaultUrl, toRef(props, 'loading'))
defineExpose({ selectSite })
const { grid, remember: rememberScroll } = useDirectoryScroll(selected, query)
// Keep the underlying browser mounted for valid native bounds; only hide its WebView.
const release = acquireWebviewSuppression()
let mounted = true
onBeforeUnmount(() => { mounted = false; release() })
onMounted(() => { void invoke('focus_main_ui').catch(() => undefined) })

const openIds = computed(() => new Set(props.openTabs))
const siteDialogVisible = ref(false)
const siteForm = reactive({ name: '', url: '' })
const siteError = ref('')
function showSiteDialog(edit = false) {
  siteForm.name = edit ? current.value?.name || '' : ''
  siteForm.url = edit ? current.value?.url || '' : ''
  siteError.value = ''
  siteDialogVisible.value = true
}
function submitSite() {
  try {
    saveSite(siteForm.url, siteForm.name)
    siteDialogVisible.value = false
  } catch (error) { siteError.value = error instanceof Error ? error.message : String(error) }
}

// One cached icon per website, not an IPC request for every tile on every filter change.
const icons = reactive(new Map<string, string | null>())
const pending = new Set<string>()
watch(groups, (list) => {
  for (const group of list) {
    const first = group.profiles[0]
    if (!first || icons.has(group.key) || pending.has(group.key)) continue
    pending.add(group.key)
    void invoke<string | null>('get_profile_icon', { id: first.id }).then(icon => {
      if (mounted) icons.set(group.key, icon)
    }).catch(() => { if (mounted) icons.set(group.key, null) }).finally(() => pending.delete(group.key))
  }
}, { immediate: true })
function iconFor(siteKey: string) { return icons.get(siteKey) || undefined }
function markIconFailed(siteKey: string) { icons.set(siteKey, null) }
function openProfile(id: string) {
  if (props.openingId) return
  emit('activate', id)
}
function removeSite() {
  removeEmptySite()
  ElMessage.success('已移除空网站分类，未修改任何账号')
}
</script>

<template>
  <section class="website-directory" aria-label="网站账号导航" :aria-busy="loading || !!openingId">
    <aside class="website-directory__sidebar" aria-label="网站分类">
      <header class="website-directory__heading">
        <strong>网站</strong>
        <button class="website-directory__columns" :title="`切换为${columns === 2 ? '单' : '双'}列`" @click="columns = columns === 2 ? 1 : 2">{{ columns }} 列</button>
        <el-button :icon="FolderAdd" circle size="small" title="添加网站" aria-label="添加网站" @click="showSiteDialog()" />
      </header>
      <button class="website-directory__all" :class="{ selected: selected === ALL_SITES }" :aria-pressed="selected === ALL_SITES" @click="selectSite(ALL_SITES)">
        <el-icon><Grid /></el-icon><span>全部账号</span><small>{{ profiles.length }}</small>
      </button>
      <nav class="website-directory__sites" :style="{ '--site-columns': columns }" aria-label="选择网站">
        <button v-for="site in groups" :key="site.key" class="website-directory__site"
          :class="{ selected: selected === site.key }" :aria-pressed="selected === site.key"
          :title="`${site.name} · ${site.key} · ${site.profiles.length} 个账号`" @click="selectSite(site.key)">
          <span>{{ site.name }}</span><small>{{ site.profiles.length }}</small>
        </button>
      </nav>
      <footer class="website-directory__sidebar-footer">
        <small>{{ groups.length }} 个网站 · 独立账号会话</small>
        <el-button :icon="Setting" text size="small" @click="emit('settings')">外观设置</el-button>
      </footer>
    </aside>

    <main class="website-directory__main">
      <header class="website-directory__toolbar">
        <el-input v-model="query" class="website-directory__search" :prefix-icon="Search" clearable
          :placeholder="current ? `搜索 ${current.name} 的账号` : '搜索账号、分组或标签'" aria-label="搜索账号">
          <template #suffix><span class="website-directory__count" aria-live="polite">{{ matches.length }}/{{ inSite.length }}</span></template>
        </el-input>
        <div class="website-directory__actions">
          <el-button :disabled="loading || !!openingId" :icon="Plus" @click="emit('create', current?.url || undefined)">新建账号</el-button>
          <el-button :disabled="loading || !!openingId" :icon="MagicStick" @click="emit('generator', current?.url || undefined)">多账号生成器</el-button>
          <el-button @click="emit('tools')">管理账号</el-button>
          <el-button v-if="activeId" :disabled="!!openingId" :icon="ArrowRight" @click="emit('close')">返回网页</el-button>
        </div>
      </header>
      <div class="website-directory__subhead">
        <div><h1>{{ current?.name || '全部账号' }}</h1><span>{{ current ? (current.url ? current.key : '请在账号设置中设置主页') : '按网站归类，一键切换账号' }}</span></div>
        <el-button v-if="current?.url" text size="small" :icon="Edit" @click="showSiteDialog(true)">编辑网站</el-button>
      </div>
      <div v-if="error" class="website-directory__empty" role="alert">
        <h2>账号列表加载失败</h2><p>{{ error }}</p><el-button :loading="loading" @click="emit('retry')">重新加载</el-button>
      </div>
      <div v-else-if="loading" class="website-directory__loading" role="status">
        <p>正在整理网站与账号…</p>
        <div class="website-directory__skeletons" aria-hidden="true">
          <div v-for="n in 8" :key="n" class="website-directory__skeleton"><i /><span /></div>
        </div>
      </div>
      <div v-else-if="!matches.length" class="website-directory__empty">
        <el-icon :size="32"><Grid /></el-icon>
        <h2>{{ query.trim() ? '没有匹配的账号' : current ? '这个网站还没有账号' : '从第一个网站开始' }}</h2>
        <p>{{ query.trim() ? '试试其他昵称或标签，或清空搜索。' : '添加网站后新建账号，已有账号会按主页地址自动归类。' }}</p>
        <el-button v-if="query.trim()" @click="query = ''">清空搜索</el-button>
        <template v-else>
          <el-button :icon="Plus" @click="emit('create', current?.url || undefined)">新建账号</el-button>
          <el-button v-if="current?.url" text @click="removeSite">移除空网站</el-button>
        </template>
      </div>
      <div v-else :key="selected" ref="grid" class="website-directory__grid" aria-label="账号列表" @scroll.passive="rememberScroll">
        <article v-for="{ profile, site, fallback } in matches" :key="profile.id" class="website-directory__tile"
          :class="{ active: profile.id === activeId, opening: profile.id === openingId }">
          <button class="website-directory__account" :disabled="!!openingId"
            :aria-label="`打开账号：${profile.name}`" :aria-pressed="profile.id === activeId"
            :title="`${profile.name}\n${site.name}${profile.note ? `\n${profile.note}` : ''}`"
            @click="openProfile(profile.id)">
            <span class="website-directory__icon" :class="{ 'chat01-icon': site.key === 'chat01.ai' }">
              <img v-if="iconFor(site.key)" :src="iconFor(site.key)" alt="" @error="markIconFailed(site.key)" />
              <span v-else>{{ fallback }}</span>
              <i v-if="openIds.has(profile.id)" class="website-directory__open-dot" title="标签页已打开" />
            </span>
            <span class="website-directory__name">{{ profile.name }}</span>
            <small v-if="openingId === profile.id">正在打开…</small>
            <small v-else-if="selected === ALL_SITES">{{ site.name }}</small>
            <small v-else-if="profile.id === activeId">当前账号</small>
          </button>
          <button class="website-directory__edit" :disabled="!!openingId" :title="`编辑 ${profile.name}`" :aria-label="`编辑账号：${profile.name}`" @click="emit('edit', profile)"><el-icon><Edit /></el-icon></button>
        </article>
      </div>
      <p v-if="openingId" class="website-directory__feedback" role="status" aria-live="polite">{{ openingId === '@creating' ? '正在创建并打开账号…' : '正在打开账号，请稍候…' }}</p>
      <footer class="website-directory__footer"><span>按账号主页归类 · 登录跳转不会改变分类</span><span>切换网站不会关闭已打开的标签页</span></footer>
    </main>

    <el-dialog v-model="siteDialogVisible" title="网站分类" width="440px" append-to-body>
      <el-form label-position="top" @submit.prevent="submitSite">
        <el-form-item label="网站名称"><el-input v-model="siteForm.name" maxlength="48" placeholder="例如：Claude（可留空自动识别）" /></el-form-item>
        <el-form-item label="网站主页" required><el-input v-model="siteForm.url" placeholder="https://claude.ai/" @keyup.enter="submitSite" /></el-form-item>
        <p class="website-directory__form-hint">相同域名归为一类；同域名再次保存可更新名称。改填其他域名会添加新分类，不会迁移现有账号。账号归属可在账号设置中修改主页。</p>
        <p v-if="siteError" class="website-directory__form-error" role="alert">{{ siteError }}</p>
      </el-form>
      <template #footer><el-button @click="siteDialogVisible = false">取消</el-button><el-button type="primary" @click="submitSite">保存网站</el-button></template>
    </el-dialog>
  </section>
</template>
