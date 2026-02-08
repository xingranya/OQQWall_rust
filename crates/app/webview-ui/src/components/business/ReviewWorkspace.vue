<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { 
  NCard, NSpace, NSelect, NInput, NButton, NTag, NEmpty, NSpin, 
  NPopconfirm, useMessage, NButtonGroup, NImage
} from 'naive-ui'
import { useReview } from '../../composables/useReview'
import { STAGE_LABELS, ACTIONS, ACTION_LABELS } from '../../api/types'
import { api } from '../../api/client'
import ReviewDetailDrawer from './ReviewDetailDrawer.vue'

const review = useReview()
const message = useMessage()

// Action Form State
const stageOptions = Object.keys(STAGE_LABELS).map(k => ({ label: STAGE_LABELS[k], value: k }))
const actionOptions = ACTIONS.map(k => ({ label: ACTION_LABELS[k], value: k }))

const batchAction = ref('approve')
const batchLoading = ref(false)

function formatTime(ms: number) {
  return new Date(ms).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}

async function handleQuickAction(reviewId: string, action: string) {
  review.actionLoading.value = true
  try {
    const payload: any = { action }
    if (action === 'comment' || action === 'reject') {
        const text = prompt('请输入评论/拒绝理由:')
        if (text === null) {
            review.actionLoading.value = false
            return
        }
        payload.comment = text
    }
    
    await api(`/api/reviews/${reviewId}/decision`, {
      method: 'POST',
      body: JSON.stringify(payload)
    })
    message.success(`执行成功: ${ACTION_LABELS[action]}`)
    await review.loadPosts()
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
                action: batchAction.value
            })
        })
        message.success('批量操作完成')
        review.selectedReviewIds.value = []
        await review.loadPosts()
    } catch(e) {
        message.error((e as Error).message)
    } finally {
        batchLoading.value = false
    }
}

onMounted(() => {
  review.loadPosts()
})
</script>

<template>
  <div class="workspace-container">
    <n-card :bordered="false" class="toolbar">
      <n-space justify="space-between" align="center" class="toolbar-flex">
        <n-space align="center" size="small" wrap>
          <n-select 
            v-model:value="review.stage.value" 
            :options="stageOptions" 
            class="stage-select"
            @update:value="review.loadPosts" 
          />
          <n-input v-model:value="review.keyword.value" placeholder="搜索..." class="search-input" />
          <n-tag :bordered="false" type="info" size="small">{{ review.posts.value.length }} 条</n-tag>
          <n-tag :bordered="false" type="warning" size="small" v-if="review.pendingCount.value > 0">待审 {{ review.pendingCount.value }}</n-tag>
        </n-space>
        
        <n-space align="center" size="small">
          <n-button size="small" @click="review.loadPosts" :loading="review.loading.value">刷新</n-button>
          <n-button size="small" @click="review.toggleSelectAll">{{ review.allSelected.value ? '取消' : '全选' }}</n-button>
        </n-space>
      </n-space>
      
      <!-- Batch Action Bar -->
      <div v-if="review.selectedReviewIds.value.length > 0" class="batch-bar">
         <span>已选 <b>{{ review.selectedReviewIds.value.length }}</b> 项</span>
         <div class="batch-actions">
            <n-select size="small" v-model:value="batchAction" :options="actionOptions" style="width: 100px" />
            <n-popconfirm @positive-click="handleBatchAction">
                <template #trigger>
                    <n-button size="small" type="primary" :loading="batchLoading">执行</n-button>
                </template>
                确定执行 {{ ACTION_LABELS[batchAction] }} 吗？
            </n-popconfirm>
         </div>
      </div>
    </n-card>

    <div class="list-content">
      <div v-if="review.loading.value && review.posts.value.length === 0" class="center-msg">
        <n-spin size="large" />
      </div>
      <n-empty v-else-if="review.filteredPosts.value.length === 0" description="没有符合条件的稿件" class="center-msg" />
      
      <div v-else class="waterfall-container">
        <div v-for="post in review.filteredPosts.value" :key="post.post_id" class="post-card-wrap">
          <n-card size="small" hoverable class="post-card">
            <template #header>
               <div class="card-header">
                 <span class="code">#{{ post.internal_code ?? '-' }}</span>
                 <n-tag size="small" :type="post.stage === 'review_pending' ? 'warning' : 'default'" class="status-tag">
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
                <!-- Preview Content -->
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
                </div>

                <div class="info-row">
                    <span>组: {{ post.group_id }}</span>
                    <span>{{ formatTime(post.created_at_ms) }}</span>
                </div>
                <div v-if="post.last_error" class="error-msg">{{ post.last_error }}</div>
            </div>

            <template #action>
                <n-space justify="end" size="small" v-if="post.review_id">
                    <n-button-group size="tiny">
                        <n-button type="primary" ghost @click.stop="handleQuickAction(post.review_id, 'approve')">过</n-button>
                        <n-button type="warning" ghost @click.stop="handleQuickAction(post.review_id, 'reject')">拒</n-button>
                        <n-button type="error" ghost @click.stop="handleQuickAction(post.review_id, 'delete')">删</n-button>
                    </n-button-group>
                </n-space>
                <div v-else class="no-action">不可操作</div>
            </template>
          </n-card>
        </div>
      </div>
    </div>

    <ReviewDetailDrawer 
        v-model:show="review.detailOpen.value" 
        :detail="review.detail.value"
        :loading="review.detailLoading.value"
    />
  </div>
</template>

<style scoped>
.workspace-container {
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
}
.toolbar {
    flex-shrink: 0;
}
.toolbar-flex {
    width: 100%;
}
.stage-select {
    width: 120px;
}
.search-input {
    width: 160px;
}

.batch-bar {
    margin-top: 12px; 
    padding: 8px 12px; 
    background: #eefdf5; 
    border-radius: 4px;
    display: flex;
    justify-content: space-between;
    align-items: center;
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
    overflow-y: auto; /* Internal scroll */
    padding-right: 4px;
}

/* Waterfall CSS Columns */
.waterfall-container {
    column-count: 1;
    column-gap: 12px;
}

@media (min-width: 640px) {
    .waterfall-container { column-count: 2; }
}
@media (min-width: 960px) {
    .waterfall-container { column-count: 3; }
}
@media (min-width: 1280px) {
    .waterfall-container { column-count: 4; }
}
@media (min-width: 1536px) {
    .waterfall-container { column-count: 5; }
}

.post-card-wrap {
    break-inside: avoid; /* Prevent card split */
    margin-bottom: 12px;
}

.center-msg {
    padding: 60px;
    display: flex;
    justify-content: center;
}
.card-header {
    display: flex;
    align-items: center;
    gap: 8px;
}
.code {
    font-weight: 700;
    font-size: 15px;
}
.status-tag {
    font-size: 10px;
    height: 18px;
    padding: 0 4px;
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
    margin-bottom: 8px;
    background: #f9f9f9;
    border-radius: 4px;
    overflow: hidden;
}
.preview-img {
    width: 100%;
    display: block;
}
:deep(.preview-img img) {
    width: 100%;
    height: auto;
    display: block;
}
.preview-text {
    padding: 8px;
    font-size: 13px;
    color: #333;
    line-height: 1.4;
    max-height: 100px;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 4;
    -webkit-box-orient: vertical;
}

.info-row {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: #888;
}
.error-msg {
    color: #d03050;
    font-size: 11px;
    margin-top: 4px;
}
.no-action {
    font-size: 12px; 
    color: #ccc; 
    text-align: center;
}
</style>