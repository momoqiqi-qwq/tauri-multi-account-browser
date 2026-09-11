<script setup lang="ts">
import { ElMessage, ElMessageBox } from 'element-plus'
import type { ProfileBatchPatch } from '../types'

const props = defineProps<{
  /** 已选中的账号 id */
  selected: string[]
  /** 可批量打的标签（来自全部账号，不受当前筛选影响） */
  knownTags: string[]
  busy: boolean
}>()

const emit = defineEmits<{
  apply: [patch: ProfileBatchPatch]
  clear: []
}>()

/** 与 Rust 侧 MAX_BULK_PROFILES 保持一致；超了后端会直接拒绝。 */
const BATCH_LIMIT = 200

/**
 * 只负责发起。**不要在这里弹成功提示** —— emit 是同步的，真正的写入在父组件里
 * 是异步的，失败了会先看到"已修改"再看到报错。成功/失败提示统一由父组件负责。
 */
function apply(patch: ProfileBatchPatch) {
  if (!props.selected.length) {
    ElMessage.warning('请先选择账号')
    return
  }
  emit('apply', patch)
}

async function setUrlMode(mode: 'inherit' | 'custom') {
  if (mode === 'inherit') {
    await apply({ url_mode: 'inherit' })
    return
  }
  const { value } = await ElMessageBox.prompt(
    '这些账号将统一使用该网址作为主页；已在旧主页上的账号也会同步过去。',
    '批量设置独立主页',
    { inputPlaceholder: 'https://example.com/', inputPattern: /^https?:\/\/\S+$/, inputErrorMessage: '请填写 http(s) 开头的完整网址' },
  ).catch(() => ({ value: undefined as string | undefined }))
  if (!value) return
  await apply({ url_mode: 'custom', default_url: value.trim() })
}

async function addTags() {
  const { value } = await ElMessageBox.prompt(
    '多个标签用英文逗号分隔；已有同名标签会自动忽略。',
    '批量添加标签',
    { inputPlaceholder: 'VIP,客服', inputValue: '' },
  ).catch(() => ({ value: undefined as string | undefined }))
  if (!value) return
  const tags = value.split(',').map((item) => item.trim()).filter(Boolean)
  if (!tags.length) {
    ElMessage.warning('没有填写标签')
    return
  }
  await apply({ tags_add: tags })
}

async function removeTags() {
  const { value } = await ElMessageBox.prompt(
    '多个标签用英文逗号分隔，大小写不敏感。',
    '批量移除标签',
    { inputPlaceholder: 'VIP,客服' },
  ).catch(() => ({ value: undefined as string | undefined }))
  if (!value) return
  const tags = value.split(',').map((item) => item.trim()).filter(Boolean)
  if (!tags.length) {
    ElMessage.warning('没有填写标签')
    return
  }
  await apply({ tags_remove: tags })
}

async function moveToGroup() {
  const { value } = await ElMessageBox.prompt(
    '留空表示移到「未分组」。',
    '批量移动分组',
    { inputPlaceholder: '分组名称' },
  ).catch(() => ({ value: undefined as string | undefined }))
  if (value === undefined) return
  await apply({ group: value.trim() })
}

async function applyQuickTag(tag: string) {
  if (!tag) return
  await apply({ tags_add: [tag] })
}
</script>

<template>
  <div class="batch-bar">
    <div class="batch-head">
      <span>已选 <strong>{{ selected.length }}</strong> 个账号</span>
      <el-button link type="primary" @click="emit('clear')">取消选择</el-button>
    </div>

    <div class="batch-actions">
      <el-button size="small" :disabled="busy" @click="setUrlMode('inherit')">主页改继承全局</el-button>
      <el-button size="small" :disabled="busy" @click="setUrlMode('custom')">主页改独立网址…</el-button>
      <el-button size="small" :disabled="busy" @click="apply({ favorite: true })">收藏</el-button>
      <el-button size="small" :disabled="busy" @click="apply({ favorite: false })">取消收藏</el-button>
      <el-button size="small" :disabled="busy" @click="addTags">加标签…</el-button>
      <el-button size="small" :disabled="busy" @click="removeTags">移除标签…</el-button>
      <el-button size="small" :disabled="busy" @click="moveToGroup">移动分组…</el-button>
      <el-button
        size="small"
        type="danger"
        :disabled="busy"
        @click="apply({ tags_set: [] })"
      >
        清空标签
      </el-button>
    </div>

    <div v-if="knownTags.length" class="batch-quick-tags">
      <small>快速打标：</small>
      <el-tag
        v-for="tag in knownTags.slice(0, 12)"
        :key="tag"
        size="small"
        effect="plain"
        class="quick-tag"
        @click="applyQuickTag(tag)"
      >
        {{ tag }}
      </el-tag>
    </div>

    <small v-if="selected.length > BATCH_LIMIT" class="batch-hint">
      单次最多修改 {{ BATCH_LIMIT }} 个账号，请分批操作。
    </small>
  </div>
</template>

<style scoped>
.batch-bar {
  border-top: 1px solid var(--el-border-color-lighter);
  padding: 10px 12px;
  background: var(--el-fill-color-lighter);
}
.batch-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 13px;
  margin-bottom: 8px;
}
.batch-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.batch-quick-tags {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  margin-top: 8px;
}
.quick-tag {
  cursor: pointer;
}
.batch-hint {
  display: block;
  margin-top: 6px;
  opacity: 0.7;
}
</style>
