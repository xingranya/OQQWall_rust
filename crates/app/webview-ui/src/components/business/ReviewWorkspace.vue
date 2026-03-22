<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import {
  NAlert,
  NButton,
  NButtonGroup,
  NCard,
  NCheckbox,
  NEmpty,
  NImage,
  NInput,
  NModal,
  NPopconfirm,
  NSelect,
  NSpace,
  NSpin,
  NSwitch,
  NTag,
  useMessage,
} from 'naive-ui'
import { api } from '../../api/client'
import { ACTION_LABELS, ACTIONS, STAGE_LABELS, type PostItem } from '../../api/types'
import { useReview } from '../../composables/useReview'
import ReviewDetailDrawer from './ReviewDetailDrawer.vue'

const review = useReview()
const message = useMessage()

const stageOptions = Object.keys(STAGE_LABELS).map((k) => ({ label: STAGE_LABELS[k], value: k }))
const actionOptions = ACTIONS.map((k) => ({ label: ACTION_LABELS[k], value: k }))
const sortOptions = [
  { label: '最新投稿优先', value: 'newest' },
  { label: '最早投稿优先', value: 'oldest' },
  { label: '编号优先', value: 'code' },
]

const batchAction = ref('approve')
const batchLoading = ref(false)
const groupFilter = ref('all')
const sortMode = ref('newest')
const autoRefresh = ref(true)
const onlyError = ref(false)
const onlyActionable = ref(false)
const lastUpdatedAt = ref<number | null>(null)

const confirmState = reactive({
  show: false,
  reviewId: '',
  action: 'approve',
  postLabel: '',
  groupId: '',
  senderId: '',
  comment: '',
})

let refreshTimer: number | null = null

function formatTime(ms: number) {
  return new Date(ms).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

const formattedUpdatedAt = computed(() => {
  if (!lastUpdatedAt.value) return '尚未刷新'
  return formatTime(lastUpdatedAt.value)
})

const groupOptions = computed(() => {
  const groups = [...new Set(review.posts.value.map((post) => post.group_id))].sort()
  return [
    { label: '全部分组', value: 'all' },
    ...groups.map((group) => ({ label: group, value: group })),
  ]
})

const visiblePosts = computed(() => {
  let items = [...review.filteredPosts.value]
  if (groupFilter.value !== 'all') {
    items = items.filter((post) => post.group_id === groupFilter.value)
  }
  if (onlyError.value) {
    items = items.filter((post) => !!post.last_error)
  }
  if (onlyActionable.value) {
    items = items.filter((post) => !!post.review_id)
  }
  switch (sortMode.value) {
    case 'oldest':
      items.sort((a, b) => a.created_at_ms - b.created_at_ms)
      break
    case 'code':
      items.sort((a, b) => (b.internal_code ?? 0) - (a.internal_code ?? 0))
      break
    default:
      items.sort((a, b) => b.created_at_ms - a.created_at_ms)
  }
  return items
})

const summaryCards = computed(() => {
  const posts = visiblePosts.value
  const errorCount = posts.filter((post) => !!post.last_error).length
  const actionableCount = posts.filter((post) => !!post.review_id).length
  const imageCount = posts.filter((post) => !!post.preview_image_url).length
  return [
    { label: '当前列表', value: posts.length, tone: 'default', hint: '筛选后的稿件数量' },
    { label: '可操作稿件', value: actionableCount, tone: 'success', hint: '当前可直接处理' },
    { label: '异常稿件', value: errorCount, tone: 'error', hint: '带最近错误信息' },
    { label: '含图稿件', value: imageCount, tone: 'warning', hint: '含渲染图或图片附件' },
  ]
})

const detailIndex = computed(() => {
  const currentPostId = review.detail.value?.post_id
  if (!currentPostId) return -1
  return visiblePosts.value.findIndex((post) => post.post_id === currentPostId)
})

function resetAutoRefresh() {
  if (refreshTimer !== null) {
    window.clearInterval(refreshTimer)
    refreshTimer = null
  }
  if (autoRefresh.value) {
    refreshTimer = window.setInterval(() => {
      loadAll()
    }, 30000)
  }
}

watch(autoRefresh, resetAutoRefresh)

async function loadAll() {
  await review.loadPosts()
  lastUpdatedAt.value = Date.now()
}

function handleResetFilters() {
  review.stage.value = 'review_pending'
  review.keyword.value = ''
  groupFilter.value = 'all'
  sortMode.value = 'newest'
  onlyError.value = false
  onlyActionable.value = false
}

function requestQuickAction(post: PostItem, action: string) {
  if (!post.review_id) {
    message.warning('当前稿件不可操作')
    return
  }
  confirmState.show = true
  confirmState.reviewId = post.review_id
  confirmState.action = action
  confirmState.postLabel = `#${post.internal_code ?? post.external_code ?? '-'}`
  confirmState.groupId = post.group_id
  confirmState.senderId = post.sender_id ?? '未知投稿人'
  confirmState.comment = ''
}

async function confirmQuickAction() {
  const payload: Record<string, unknown> = { action: confirmState.action }
  if (confirmState.action === 'reject' && confirmState.comment.trim()) {
    payload.comment = confirmState.comment.trim()
  }

  review.actionLoading.value = true
  try {
    await api(`/api/reviews/${confirmState.reviewId}/decision`, {
      method: 'POST',
      body: JSON.stringify(payload),
    })
    message.success(`执行成功: ${ACTION_LABELS[confirmState.action]}`)
    confirmState.show = false
    await loadAll()
    await review.refreshDetail()
  } catch (e) {
    message.error((e as Error).message)
  } finally {
    review.actionLoading.value = false
  }
}

async function handleBatchAction() {
  if (review.selectedReviewIds.value.length === 0) return
  batchLoading.value = true
  try {
    await api('/api/reviews/batch', {
      method: 'POST',
      body: JSON.stringify({
        review_ids: review.selectedReviewIds.value,
        action: batchAction.value,
      }),
    })
    message.success('批量操作完成')
    review.selectedReviewIds.value = []
    await loadAll()
    await review.refreshDetail()
  } catch (e) {
    message.error((e as Error).message)
  } finally {
    batchLoading.value = false
  }
}

async function handleDrawerRefresh() {
  await loadAll()
  await review.refreshDetail()
}

async function openAdjacentDetail(offset: number) {
  const nextIndex = detailIndex.value + offset
  if (nextIndex < 0 || nextIndex >= visiblePosts.value.length) return
  await review.openDetail(visiblePosts.value[nextIndex].post_id)
}

onMounted(async () => {
  await loadAll()
  resetAutoRefresh()
})

onBeforeUnmount(() => {
  if (refreshTimer !== null) {
    window.clearInterval(refreshTimer)
  }
})
</script>

<template>
  <div class="workspace-container">
    <section class="hero-panel">
      <div class="hero-copy">
        <span class="hero-kicker">稿件审核</span>
        <h1>按状态查看稿件并执行审核操作。</h1>
        <p>支持分组筛选、搜索、批量处理和详情页操作。</p>
      </div>
      <div class="hero-metrics">
        <div v-for="card in summaryCards" :key="card.label" class="metric-card" :data-tone="card.tone">
          <span>{{ card.label }}</span>
          <strong>{{ card.value }}</strong>
          <small>{{ card.hint }}</small>
        </div>
      </div>
    </section>

    <n-card :bordered="false" class="toolbar-card">
      <div class="toolbar-main">
        <div class="toolbar-grid">
          <n-select
            v-model:value="review.stage.value"
            :options="stageOptions"
            class="stage-select"
            @update:value="loadAll"
          />
          <n-select v-model:value="groupFilter" :options="groupOptions" class="group-select" />
          <n-select v-model:value="sortMode" :options="sortOptions" class="sort-select" />
          <n-input
            v-model:value="review.keyword.value"
            placeholder="搜索编号、分组、投稿人、错误或预览文本"
            class="search-input"
            clearable
          />
        </div>

        <div class="toolbar-actions">
          <div class="toolbar-tags">
            <n-tag :bordered="false" type="info" round>{{ visiblePosts.length }} 条结果</n-tag>
            <n-tag :bordered="false" type="warning" round v-if="review.pendingCount.value > 0">
              待审 {{ review.pendingCount.value }}
            </n-tag>
            <n-tag :bordered="false" type="success" round v-if="review.selectedReviewIds.value.length > 0">
              已选 {{ review.selectedReviewIds.value.length }}
            </n-tag>
            <n-tag :bordered="false" round>上次刷新 {{ formattedUpdatedAt }}</n-tag>
          </div>

          <div class="toolbar-flags">
            <n-checkbox v-model:checked="onlyActionable">仅看可操作</n-checkbox>
            <n-checkbox v-model:checked="onlyError">仅看异常</n-checkbox>
            <div class="toolbar-switch">
              <span>自动刷新</span>
              <n-switch v-model:value="autoRefresh" />
            </div>
          </div>

          <div class="toolbar-buttons">
            <n-button size="small" @click="handleResetFilters">重置筛选</n-button>
            <n-button size="small" @click="loadAll" :loading="review.loading.value">刷新</n-button>
            <n-button size="small" @click="review.toggleSelectAll">{{ review.allSelected.value ? '取消全选' : '全选' }}</n-button>
          </div>
        </div>
      </div>

      <div v-if="review.selectedReviewIds.value.length > 0" class="batch-bar">
        <span>已选择 <b>{{ review.selectedReviewIds.value.length }}</b> 条可操作稿件</span>
        <div class="batch-actions">
          <n-select size="small" v-model:value="batchAction" :options="actionOptions" style="width: 140px" />
          <n-popconfirm @positive-click="handleBatchAction">
            <template #trigger>
              <n-button size="small" type="primary" :loading="batchLoading">执行批量动作</n-button>
            </template>
            确定批量执行 {{ ACTION_LABELS[batchAction] }} 吗？
          </n-popconfirm>
        </div>
      </div>
    </n-card>

    <div class="list-content">
      <div v-if="review.loading.value && review.posts.value.length === 0" class="center-msg">
        <n-spin size="large" />
      </div>
      <n-empty v-else-if="visiblePosts.length === 0" description="没有符合条件的稿件" class="center-msg" />

      <div v-else class="grid-container">
        <article v-for="post in visiblePosts" :key="post.post_id" class="post-card-wrap">
          <n-card size="small" hoverable class="post-card" :bordered="false">
            <template #header>
              <div class="card-header">
                <div>
                  <span class="code">#{{ post.internal_code ?? post.external_code ?? '-' }}</span>
                  <p class="card-subhead">{{ post.sender_id ?? '未知投稿人' }}</p>
                </div>
                <n-tag size="small" :type="post.stage === 'review_pending' ? 'warning' : 'default'" round class="status-tag">
                  {{ STAGE_LABELS[post.stage] ?? post.stage }}
                </n-tag>
              </div>
            </template>

            <template #header-extra>
              <input
                v-if="post.review_id"
                type="checkbox"
                :checked="review.selectedReviewIds.value.includes(post.review_id)"
                @click.stop="review.toggleOneSelection(post.review_id!, !review.selectedReviewIds.value.includes(post.review_id!))"
                class="checkbox"
              />
            </template>

            <div class="card-body" @click="review.openDetail(post.post_id)">
              <div class="preview-area">
                <n-image
                  v-if="post.preview_image_url"
                  :src="post.preview_image_url"
                  class="preview-img"
                  preview-disabled
                />
                <div v-if="post.preview_text" class="preview-text">
                  {{ post.preview_text }}
                </div>
                <div v-else class="preview-fallback">
                  <span>{{ post.last_error ? '该稿件存在异常信息' : '点击查看稿件详情' }}</span>
                </div>
              </div>

              <div class="meta-row">
                <n-tag size="small" :bordered="false" round>{{ post.group_id }}</n-tag>
                <span>{{ formatTime(post.created_at_ms) }}</span>
              </div>
              <div v-if="post.last_error" class="error-msg">{{ post.last_error }}</div>
            </div>

            <template #action>
              <n-space justify="space-between" align="center" size="small" v-if="post.review_id">
                <span class="action-tip">点击卡片查看详情</span>
                <n-button-group size="tiny">
                  <n-button type="primary" ghost @click.stop="requestQuickAction(post, 'approve')">通过</n-button>
                  <n-button type="warning" ghost @click.stop="requestQuickAction(post, 'reject')">拒绝</n-button>
                  <n-button type="error" ghost @click.stop="requestQuickAction(post, 'delete')">删除</n-button>
                </n-button-group>
              </n-space>
              <div v-else class="no-action">当前阶段暂无可执行动作</div>
            </template>
          </n-card>
        </article>
      </div>
    </div>

    <ReviewDetailDrawer
      v-model:show="review.detailOpen.value"
      :detail="review.detail.value"
      :loading="review.detailLoading.value"
      :has-prev="detailIndex > 0"
      :has-next="detailIndex >= 0 && detailIndex < visiblePosts.length - 1"
      @refresh="handleDrawerRefresh"
      @prev="openAdjacentDetail(-1)"
      @next="openAdjacentDetail(1)"
    />

    <n-modal v-model:show="confirmState.show" preset="card" class="confirm-modal" :mask-closable="false">
      <div class="confirm-head">
        <span class="confirm-kicker">确认操作</span>
        <h3>{{ ACTION_LABELS[confirmState.action] }} {{ confirmState.postLabel }}</h3>
      </div>
      <p class="confirm-meta">分组：{{ confirmState.groupId }} · 投稿人：{{ confirmState.senderId }}</p>
      <n-input
        v-if="confirmState.action === 'reject'"
        v-model:value="confirmState.comment"
        type="textarea"
        :autosize="{ minRows: 3, maxRows: 5 }"
        placeholder="可填写拒绝说明"
        style="margin-bottom: 14px"
      />
      <n-alert type="warning" :bordered="false">
        操作确认后会立即提交到后端处理。
      </n-alert>
      <div class="confirm-actions">
        <n-button @click="confirmState.show = false">取消</n-button>
        <n-button type="primary" :loading="review.actionLoading.value" @click="confirmQuickAction">确认执行</n-button>
      </div>
    </n-modal>
  </div>
</template>

<style scoped>
.workspace-container {
  display: flex;
  flex-direction: column;
  gap: 18px;
  height: 100%;
}

.hero-panel {
  display: grid;
  grid-template-columns: minmax(0, 1.1fr) minmax(300px, 400px);
  gap: 18px;
  align-items: stretch;
}

.hero-copy,
.hero-metrics {
  border-radius: 26px;
  padding: 24px;
  background: rgba(255, 250, 242, 0.88);
  border: 1px solid var(--app-border-strong);
  box-shadow: var(--app-shadow-soft);
}

.hero-kicker {
  display: inline-block;
  margin-bottom: 12px;
  color: rgba(38, 29, 23, 0.46);
  letter-spacing: 0.14em;
  text-transform: uppercase;
  font-size: 11px;
}

.hero-copy h1 {
  margin: 0;
  max-width: 12ch;
  font-family: Georgia, "Times New Roman", serif;
  font-size: clamp(28px, 3.2vw, 40px);
  line-height: 1.12;
  color: #261d17;
}

.hero-copy p {
  max-width: 42rem;
  margin: 14px 0 0;
  color: rgba(38, 29, 23, 0.64);
  line-height: 1.72;
}

.hero-metrics {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
  align-content: start;
}

.metric-card {
  padding: 16px 18px;
  border-radius: 20px;
  background: rgba(244, 237, 226, 0.8);
  border: 1px solid rgba(75, 62, 53, 0.08);
  min-height: 126px;
}

.metric-card span,
.metric-card small {
  display: block;
}

.metric-card span {
  color: rgba(38, 29, 23, 0.58);
  font-size: 12px;
}

.metric-card strong {
  display: block;
  margin: 12px 0 8px;
  font-size: 30px;
  line-height: 1;
  color: #261d17;
}

.metric-card small {
  color: rgba(38, 29, 23, 0.54);
  line-height: 1.6;
}

.metric-card[data-tone="success"] {
  box-shadow: inset 0 0 0 1px rgba(31, 143, 106, 0.14);
}

.metric-card[data-tone="error"] {
  box-shadow: inset 0 0 0 1px rgba(184, 77, 58, 0.14);
}

.metric-card[data-tone="warning"] {
  box-shadow: inset 0 0 0 1px rgba(200, 122, 42, 0.14);
}

.toolbar-card {
  flex-shrink: 0;
  border-radius: 26px;
  background: rgba(255, 250, 242, 0.94);
  box-shadow: var(--app-shadow);
}

.toolbar-main {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.toolbar-grid {
  display: grid;
  grid-template-columns: 160px 160px 160px minmax(260px, 1fr);
  gap: 12px;
}

.toolbar-actions {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  gap: 14px;
  align-items: center;
}

.toolbar-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.toolbar-flags {
  display: flex;
  align-items: center;
  gap: 16px;
  color: rgba(38, 29, 23, 0.72);
}

.toolbar-switch {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  color: rgba(38, 29, 23, 0.72);
  font-size: 13px;
}

.toolbar-buttons {
  display: flex;
  gap: 8px;
}

.stage-select,
.group-select,
.sort-select,
.search-input {
  width: 100%;
}

.batch-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 14px 16px;
  border-radius: 18px;
  background: linear-gradient(90deg, rgba(31, 143, 106, 0.08), rgba(53, 94, 123, 0.04));
  border: 1px solid rgba(31, 143, 106, 0.12);
  color: #2a211b;
  font-size: 13px;
}

.batch-actions {
  display: flex;
  gap: 8px;
  align-items: center;
}

.list-content {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-right: 4px;
}

.grid-container {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 16px;
}

.post-card-wrap {
  min-width: 0;
}

.center-msg {
  padding: 60px;
  display: flex;
  justify-content: center;
  border-radius: 24px;
  background: rgba(255, 250, 242, 0.88);
  border: 1px solid var(--app-border-strong);
}

.post-card {
  height: 100%;
  border-radius: 22px;
  overflow: hidden;
  background: rgba(255, 250, 242, 0.96);
  box-shadow: var(--app-shadow-soft);
  transition: transform 0.18s ease, box-shadow 0.18s ease;
}

.post-card:hover {
  transform: translateY(-3px);
  box-shadow: 0 20px 34px rgba(22, 16, 11, 0.14);
}

.card-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.code {
  font-size: 24px;
  font-family: Georgia, "Times New Roman", serif;
  font-weight: 700;
  color: #201812;
}

.card-subhead {
  margin: 6px 0 0;
  font-size: 12px;
  color: rgba(32, 24, 18, 0.62);
}

.status-tag {
  flex-shrink: 0;
}

.checkbox {
  cursor: pointer;
  transform: scale(1.2);
}

.card-body {
  cursor: pointer;
  padding-bottom: 8px;
}

.preview-area {
  margin-bottom: 12px;
  border-radius: 16px;
  overflow: hidden;
  background:
    linear-gradient(160deg, rgba(28, 26, 24, 0.05), rgba(28, 26, 24, 0.01)),
    linear-gradient(135deg, rgba(31, 143, 106, 0.06), rgba(200, 122, 42, 0.05));
  border: 1px solid rgba(32, 24, 18, 0.06);
}

.preview-img {
  width: 100%;
  display: block;
}

:deep(.preview-img img) {
  width: 100%;
  height: 220px;
  object-fit: cover;
  display: block;
}

.preview-text {
  padding: 14px;
  font-size: 13px;
  color: #332821;
  line-height: 1.65;
  min-height: 88px;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 4;
  -webkit-box-orient: vertical;
}

.preview-fallback {
  min-height: 120px;
  display: grid;
  place-items: center;
  padding: 16px;
  text-align: center;
  color: rgba(51, 40, 33, 0.56);
  line-height: 1.7;
}

.meta-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: rgba(42, 33, 27, 0.62);
}

.error-msg {
  margin-top: 12px;
  padding: 10px 12px;
  border-radius: 14px;
  background: rgba(184, 77, 58, 0.08);
  color: #9c3427;
  font-size: 12px;
  line-height: 1.6;
}

.no-action {
  font-size: 12px;
  color: rgba(42, 33, 27, 0.4);
  text-align: center;
}

.action-tip {
  color: rgba(42, 33, 27, 0.52);
  font-size: 12px;
}

.confirm-modal {
  max-width: 520px;
}

.confirm-head h3 {
  margin: 8px 0 6px;
  color: #261d17;
}

.confirm-kicker {
  font-size: 11px;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: rgba(38, 29, 23, 0.46);
}

.confirm-meta {
  margin: 0 0 14px;
  color: rgba(38, 29, 23, 0.62);
  line-height: 1.7;
}

.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 16px;
}

@media (max-width: 1180px) {
  .hero-panel {
    grid-template-columns: 1fr;
  }

  .toolbar-actions {
    grid-template-columns: 1fr;
  }

  .toolbar-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 760px) {
  .hero-copy,
  .hero-metrics,
  .toolbar-card {
    border-radius: 22px;
  }

  .hero-copy {
    padding: 20px;
  }

  .hero-copy h1 {
    max-width: none;
  }

  .hero-metrics {
    padding: 18px;
    grid-template-columns: 1fr 1fr;
  }

  .toolbar-grid {
    grid-template-columns: 1fr;
  }

  .toolbar-flags,
  .toolbar-buttons,
  .batch-bar {
    flex-direction: column;
    align-items: stretch;
  }

  .grid-container {
    grid-template-columns: 1fr;
  }
}
</style>
