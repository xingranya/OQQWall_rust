<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { NDrawer, NDrawerContent, NDescriptions, NDescriptionsItem, NImage, NTag, NSpin, NEmpty } from 'naive-ui'
import { STAGE_LABELS, type PostDetail } from '../../api/types'

const props = defineProps<{
  show: boolean
  loading: boolean
  detail: PostDetail | null
}>()

const emit = defineEmits<{
  (e: 'update:show', v: boolean): void
}>()

const windowWidth = ref(window.innerWidth)

const updateWidth = () => {
    windowWidth.value = window.innerWidth
}

onMounted(() => window.addEventListener('resize', updateWidth))
onUnmounted(() => window.removeEventListener('resize', updateWidth))

const isMobile = computed(() => windowWidth.value < 640)

const drawerWidth = computed(() => {
    return isMobile.value ? '100%' : 600
})

const visible = computed({
  get: () => props.show,
  set: (v) => emit('update:show', v),
})

function formatTime(ms: number) {
  return new Date(ms).toLocaleString('zh-CN')
}

function renderImageUrl(blockRef: { reference_type: 'blob_id' | 'remote_url'; reference: string }) {
  return blockRef.reference_type === 'blob_id' ? '/api/blobs/' + blockRef.reference : blockRef.reference
}
</script>

<template>
  <n-drawer v-model:show="visible" :width="drawerWidth" placement="right">
    <n-drawer-content title="稿件详情" closable>
      <div v-if="loading" class="loading-wrap">
        <n-spin size="large" />
      </div>
      <div v-else-if="detail" class="detail-container">
        <n-descriptions 
            bordered 
            column="1" 
            size="small" 
            label-placement="left" 
            :label-style="{ width: isMobile ? '70px' : '90px' }"
        >
          <n-descriptions-item label="编号">#{{ detail.review_code ?? detail.external_code ?? '-' }}</n-descriptions-item>
          <n-descriptions-item label="状态">
            <n-tag :type="detail.stage === 'review_pending' ? 'warning' : 'default'">{{ STAGE_LABELS[detail.stage] ?? detail.stage }}</n-tag>
          </n-descriptions-item>
          <n-descriptions-item label="投稿人">{{ detail.sender_id ?? '未知' }}</n-descriptions-item>
          <n-descriptions-item label="时间">{{ formatTime(detail.created_at_ms) }}</n-descriptions-item>
          <n-descriptions-item label="匿名">{{ detail.is_anonymous ? '是' : '否' }}</n-descriptions-item>
          <n-descriptions-item label="安全">{{ detail.is_safe ? '是' : '否' }}</n-descriptions-item>
          <n-descriptions-item label="Session">
              <span class="session-text">{{ detail.session_id }}</span>
          </n-descriptions-item>
        </n-descriptions>

        <div v-if="detail.render_png_blob_id" class="section">
          <h4>渲染预览</h4>
          <n-image
            :src="'/api/blobs/' + detail.render_png_blob_id"
            class="full-width-image"
          />
        </div>

        <div class="section">
          <h4>内容块</h4>
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
              <a v-else :href="renderImageUrl(block)" target="_blank" class="download-link">下载/预览附件</a>
            </template>
          </div>
        </div>

        <div v-if="detail.last_error" class="section error">
          <h4>最近错误</h4>
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
  padding: 40px;
}
.detail-container {
  display: flex;
  flex-direction: column;
  gap: 20px;
  padding-bottom: 40px;
}
.section h4 {
  margin: 0 0 10px;
  font-size: 14px;
  color: #666;
  border-left: 3px solid #18a058;
  padding-left: 8px;
}

/* 全屏宽度自适应图片样式 */
.full-width-image {
  width: 100%;
  display: block;
}
:deep(.full-width-image img) {
  width: 100%;
  height: auto;
  display: block;
  border-radius: 8px;
}

.block-item {
  background: #f9f9f9;
  border: 1px solid #eee;
  padding: 12px;
  border-radius: 8px;
  margin-bottom: 8px;
  overflow: hidden;
}
.text-content {
  white-space: pre-wrap;
  font-family: v-sans;
  word-break: break-all;
}
.media-header {
  font-size: 12px;
  color: #999;
  margin-bottom: 8px;
}
.download-link {
  color: #18a058;
  text-decoration: none;
}
.section.error pre {
  color: #d03050;
  background: #fff0f0;
  padding: 10px;
  border-radius: 4px;
  white-space: pre-wrap;
  word-break: break-all;
}
.session-text {
    word-break: break-all;
    font-size: 12px;
    font-family: monospace;
}
</style>
