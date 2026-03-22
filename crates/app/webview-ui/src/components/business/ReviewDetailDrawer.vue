<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import {
  NButton,
  NDescriptions,
  NDescriptionsItem,
  NDivider,
  NDrawer,
  NDrawerContent,
  NEmpty,
  NForm,
  NFormItem,
  NImage,
  NInput,
  NInputNumber,
  NSelect,
  NSpin,
  NTag,
  useMessage,
} from 'naive-ui'
import { api } from '../../api/client'
import { ACTION_LABELS, ACTIONS, STAGE_LABELS, type PostDetail } from '../../api/types'

const props = defineProps<{
  show: boolean
  loading: boolean
  detail: PostDetail | null
}>()

const emit = defineEmits<{
  (e: 'update:show', v: boolean): void
  (e: 'refresh'): void
}>()

const message = useMessage()
const windowWidth = ref(window.innerWidth)
const submitting = ref(false)

const actionForm = reactive({
  action: 'approve',
  comment: '',
  text: '',
  delay_ms: 180000,
  quick_reply_key: '',
  target_review_code: null as number | null,
})

const actionOptions = ACTIONS.map((k) => ({ label: ACTION_LABELS[k], value: k }))

const updateWidth = () => {
  windowWidth.value = window.innerWidth
}

onMounted(() => window.addEventListener('resize', updateWidth))
onUnmounted(() => window.removeEventListener('resize', updateWidth))

const isMobile = computed(() => windowWidth.value < 640)
const drawerWidth = computed(() => (isMobile.value ? '100%' : 760))

const visible = computed({
  get: () => props.show,
  set: (v) => emit('update:show', v),
})

watch(
  () => props.detail?.post_id,
  () => {
    actionForm.action = 'approve'
    actionForm.comment = ''
    actionForm.text = ''
    actionForm.delay_ms = 180000
    actionForm.quick_reply_key = ''
    actionForm.target_review_code = null
  },
)

function formatTime(ms: number) {
  return new Date(ms).toLocaleString('zh-CN')
}

function renderImageUrl(blockRef: { reference_type: 'blob_id' | 'remote_url'; reference: string }) {
  return blockRef.reference_type === 'blob_id' ? '/api/blobs/' + blockRef.reference : blockRef.reference
}

const actionHelp = computed(() => {
  switch (actionForm.action) {
    case 'defer':
      return '稿件会在指定时间后再次进入处理列表。'
    case 'quick_reply':
      return '填写已配置的快捷回复键名。'
    case 'merge':
      return '将当前稿件合并到目标审核编号。'
    case 'toggle_anonymous':
      return '切换当前稿件的匿名状态。'
    case 'rerender':
      return '重新生成当前稿件的渲染图。'
    case 'reply':
      return '向投稿人发送回复。'
    case 'comment':
      return '为当前稿件添加备注。'
    case 'blacklist':
      return '将投稿人加入黑名单。'
    default:
      return '选择动作后，下方会显示对应参数。'
  }
})

const blockStats = computed(() => {
  const blocks = props.detail?.blocks ?? []
  const textCount = blocks.filter((block) => block.kind === 'text').length
  const attachmentCount = blocks.length - textCount
  return { textCount, attachmentCount }
})

function buildPayload(action: string) {
  const payload: Record<string, unknown> = { action }

  if (['reject', 'blacklist'].includes(action)) {
    const comment = actionForm.comment.trim()
    if (!comment) {
      throw new Error('请填写处理说明')
    }
    payload.comment = comment
  }

  if (action === 'comment') {
    const text = actionForm.text.trim() || actionForm.comment.trim()
    if (!text) {
      throw new Error('请填写评论内容')
    }
    payload.text = text
  }

  if (action === 'reply') {
    const text = actionForm.text.trim()
    if (!text) {
      throw new Error('请填写回复内容')
    }
    payload.text = text
  }

  if (action === 'defer') {
    if (!actionForm.delay_ms || actionForm.delay_ms <= 0) {
      throw new Error('请填写大于 0 的暂缓时长')
    }
    payload.delay_ms = actionForm.delay_ms
  }

  if (action === 'quick_reply') {
    const key = actionForm.quick_reply_key.trim()
    if (!key) {
      throw new Error('请填写快捷回复键名')
    }
    payload.quick_reply_key = key
  }

  if (action === 'merge') {
    if (!actionForm.target_review_code) {
      throw new Error('请填写目标审核编号')
    }
    payload.target_review_code = actionForm.target_review_code
  }

  return payload
}

async function handleExecute(actionOverride?: string) {
  if (!props.detail?.review_id) {
    message.error('当前稿件无法操作（无 review_id）')
    return
  }

  const action = actionOverride ?? actionForm.action

  let payload: Record<string, unknown>
  try {
    payload = buildPayload(action)
  } catch (error) {
    message.error((error as Error).message)
    return
  }

  submitting.value = true
  try {
    await api(`/api/reviews/${props.detail.review_id}/decision`, {
      method: 'POST',
      body: JSON.stringify(payload),
    })
    message.success(`执行成功: ${ACTION_LABELS[action]}`)
    emit('refresh')
    if (['approve', 'reject', 'delete', 'immediate'].includes(action)) {
      emit('update:show', false)
    }
  } catch (e) {
    message.error((e as Error).message)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <n-drawer v-model:show="visible" :width="drawerWidth" placement="right" :trap-focus="false">
    <n-drawer-content title="稿件详情" closable native-scrollbar>
      <div v-if="loading" class="loading-wrap">
        <n-spin size="large" />
      </div>
      <div v-else-if="detail" class="detail-wrapper">
        <section class="detail-hero">
          <div>
            <span class="detail-kicker">稿件信息</span>
            <h2>#{{ detail.review_code ?? detail.external_code ?? '-' }}</h2>
            <p>{{ detail.sender_id ?? '未知投稿人' }} · {{ formatTime(detail.created_at_ms) }}</p>
          </div>
          <div class="hero-tags">
            <n-tag :type="detail.stage === 'review_pending' ? 'warning' : 'default'" round>{{ STAGE_LABELS[detail.stage] ?? detail.stage }}</n-tag>
            <n-tag :type="detail.is_safe ? 'success' : 'error'" round>{{ detail.is_safe ? '安全' : '待核查' }}</n-tag>
            <n-tag :type="detail.is_anonymous ? 'info' : 'default'" round>{{ detail.is_anonymous ? '匿名' : '非匿名' }}</n-tag>
          </div>
        </section>

        <section class="action-panel">
          <div class="action-panel-head">
            <div>
              <span class="panel-kicker">审核操作</span>
              <h3>常用操作可直接执行，其他操作按需填写参数。</h3>
            </div>
            <div class="quick-actions">
              <n-button type="primary" @click="handleExecute('approve')" :loading="submitting">通过</n-button>
              <n-button type="warning" ghost @click="handleExecute('reject')" :loading="submitting">拒绝</n-button>
              <n-button type="error" ghost @click="handleExecute('delete')" :loading="submitting">删除</n-button>
              <n-button ghost @click="handleExecute('immediate')" :loading="submitting">立即发送</n-button>
              <n-button ghost @click="handleExecute('rerender')" :loading="submitting">重渲染</n-button>
            </div>
          </div>

          <n-divider />

          <n-form label-placement="top" class="advanced-form">
            <n-form-item label="动作类型">
              <n-select v-model:value="actionForm.action" :options="actionOptions" />
            </n-form-item>
            <p class="action-help">{{ actionHelp }}</p>

            <n-form-item v-if="['reject', 'blacklist'].includes(actionForm.action)" label="处理说明">
              <n-input
                v-model:value="actionForm.comment"
                type="textarea"
                :autosize="{ minRows: 3, maxRows: 5 }"
                placeholder="请输入拒绝原因或拉黑说明"
              />
            </n-form-item>

            <n-form-item
              v-if="['comment', 'reply'].includes(actionForm.action)"
              :label="actionForm.action === 'reply' ? '回复内容' : '评论内容'"
            >
              <n-input
                v-model:value="actionForm.text"
                type="textarea"
                :autosize="{ minRows: 3, maxRows: 6 }"
                placeholder="请输入文本内容"
              />
            </n-form-item>

            <n-form-item v-if="actionForm.action === 'defer'" label="暂缓时长（毫秒）">
              <n-input-number v-model:value="actionForm.delay_ms" :min="1000" :step="60000" style="width: 100%" />
            </n-form-item>

            <n-form-item v-if="actionForm.action === 'quick_reply'" label="快捷回复键名">
              <n-input v-model:value="actionForm.quick_reply_key" placeholder="请输入快捷回复键名" />
            </n-form-item>

            <n-form-item v-if="actionForm.action === 'merge'" label="目标审核编号">
              <n-input-number v-model:value="actionForm.target_review_code" :min="1" style="width: 100%" />
            </n-form-item>

            <n-button type="primary" block :loading="submitting" @click="handleExecute()">
              执行当前动作
            </n-button>
          </n-form>
        </section>

        <n-descriptions
          bordered
          column="1"
          size="small"
          label-placement="left"
          :label-style="{ width: isMobile ? '76px' : '96px' }"
          class="info-panel"
        >
          <n-descriptions-item label="组别">{{ detail.group_id }}</n-descriptions-item>
          <n-descriptions-item label="投稿人">{{ detail.sender_id ?? '未知' }}</n-descriptions-item>
          <n-descriptions-item label="时间">{{ formatTime(detail.created_at_ms) }}</n-descriptions-item>
          <n-descriptions-item label="文本块">{{ blockStats.textCount }}</n-descriptions-item>
          <n-descriptions-item label="附件块">{{ blockStats.attachmentCount }}</n-descriptions-item>
          <n-descriptions-item label="会话 ID">
            <span class="session-text">{{ detail.session_id }}</span>
          </n-descriptions-item>
        </n-descriptions>

        <div v-if="detail.render_png_blob_id" class="section">
          <div class="section-head">
            <span class="section-kicker">渲染预览</span>
            <h4>预览图</h4>
          </div>
          <n-image :src="'/api/blobs/' + detail.render_png_blob_id" class="full-width-image" />
        </div>

        <div class="section">
          <div class="section-head">
            <span class="section-kicker">稿件内容</span>
            <h4>内容块</h4>
          </div>
          <div v-for="(block, idx) in detail.blocks" :key="idx" class="block-item">
            <template v-if="block.kind === 'text'">
              <div class="text-content">{{ block.text }}</div>
            </template>
            <template v-else>
              <div class="media-header">{{ block.media_kind }} · {{ block.reference_type }}</div>
              <n-image
                v-if="block.media_kind === 'image'"
                :src="renderImageUrl(block)"
                class="full-width-image"
              />
              <a v-else :href="renderImageUrl(block)" target="_blank" class="download-link">打开附件</a>
            </template>
          </div>
        </div>

        <div v-if="detail.last_error" class="section error">
          <div class="section-head">
            <span class="section-kicker">异常记录</span>
            <h4>最近错误</h4>
          </div>
          <pre>{{ detail.last_error }}</pre>
        </div>
      </div>
      <n-empty v-else description="暂无数据" />
    </n-drawer-content>
  </n-drawer>
</template>

<style scoped>
.loading-wrap {
  display: flex;
  justify-content: center;
  padding: 48px;
}

.detail-wrapper {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding-bottom: 24px;
}

.detail-hero,
.action-panel,
.info-panel,
.section {
  border-radius: 24px;
  overflow: hidden;
}

.detail-hero {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 18px;
  padding: 24px;
  background: linear-gradient(135deg, #1f8f6a, #274f5a 70%, #1e2328);
  color: #fff7ea;
}

.detail-kicker,
.panel-kicker,
.section-kicker {
  display: inline-block;
  margin-bottom: 10px;
  font-size: 11px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
}

.detail-hero h2 {
  margin: 0;
  font-family: Georgia, "Times New Roman", serif;
  font-size: clamp(34px, 6vw, 52px);
  line-height: 1;
}

.detail-hero p {
  margin: 10px 0 0;
  color: rgba(255, 247, 234, 0.76);
}

.hero-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  justify-content: flex-end;
}

.action-panel {
  padding: 22px;
  background: rgba(255, 248, 238, 0.96);
  box-shadow: 0 20px 36px rgba(17, 14, 12, 0.1);
}

.action-panel-head {
  display: flex;
  justify-content: space-between;
  gap: 14px;
  align-items: flex-start;
}

.action-panel-head h3 {
  margin: 0;
  font-size: 22px;
  line-height: 1.25;
  color: #261d17;
}

.quick-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  justify-content: flex-end;
}

.action-help {
  margin: -2px 0 14px;
  color: rgba(38, 29, 23, 0.62);
  line-height: 1.7;
  font-size: 13px;
}

.advanced-form {
  display: grid;
  gap: 2px;
}

.info-panel {
  background: rgba(255, 248, 238, 0.92);
  box-shadow: 0 18px 32px rgba(17, 14, 12, 0.08);
}

.section {
  padding: 22px;
  background: rgba(255, 248, 238, 0.92);
  box-shadow: 0 18px 32px rgba(17, 14, 12, 0.08);
}

.section-head {
  margin-bottom: 14px;
}

.section-head h4 {
  margin: 0;
  font-size: 22px;
  color: #261d17;
}

.full-width-image {
  width: 100%;
  display: block;
}

:deep(.full-width-image img) {
  width: 100%;
  height: auto;
  display: block;
  border-radius: 18px;
}

.block-item {
  background: rgba(28, 26, 24, 0.04);
  border: 1px solid rgba(28, 26, 24, 0.08);
  padding: 14px;
  border-radius: 18px;
  margin-bottom: 10px;
  overflow: hidden;
}

.text-content {
  white-space: pre-wrap;
  word-break: break-word;
  line-height: 1.8;
  color: #2f261f;
}

.media-header {
  font-size: 12px;
  color: rgba(47, 38, 31, 0.52);
  margin-bottom: 8px;
  letter-spacing: 0.04em;
}

.download-link {
  color: #1f8f6a;
  text-decoration: none;
}

.section.error pre {
  color: #9c3427;
  background: rgba(184, 77, 58, 0.08);
  padding: 14px;
  border-radius: 16px;
  white-space: pre-wrap;
  word-break: break-all;
}

.session-text {
  word-break: break-all;
  font-size: 12px;
  font-family: "Fira Code", "Cascadia Code", monospace;
}

@media (max-width: 760px) {
  .detail-hero,
  .action-panel-head {
    flex-direction: column;
  }

  .hero-tags,
  .quick-actions {
    justify-content: flex-start;
  }
}
</style>
