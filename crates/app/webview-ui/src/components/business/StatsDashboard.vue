<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { NCard, NGrid, NGridItem, NStatistic, NNumberAnimation, NProgress, NSpace, NSpin, useMessage } from 'naive-ui'
import { api } from '../../api/client'
import type { StatsResponse } from '../../api/types'
import { STAGE_LABELS } from '../../api/types'

const message = useMessage()
const loading = ref(false)
const stats = ref<StatsResponse | null>(null)

async function loadStats() {
  loading.value = true
  try {
    stats.value = await api<StatsResponse>('/api/stats')
  } catch (e) {
    message.error((e as Error).message)
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  loadStats()
})
</script>

<template>
  <div class="stats-container">
    <div class="header">
        <h2>数据统计</h2>
        <button class="refresh-btn" @click="loadStats">刷新</button>
    </div>

    <div v-if="loading && !stats" class="loading-wrap">
        <n-spin size="large" />
    </div>

    <div v-else-if="stats" class="dashboard">
        <n-grid x-gap="12" y-gap="12" cols="1 s:2 m:3" responsive="screen">
            <n-grid-item>
                <n-card>
                    <n-statistic label="今日投稿">
                        <n-number-animation :from="0" :to="stats.today_count" />
                    </n-statistic>
                </n-card>
            </n-grid-item>
            <n-grid-item>
                <n-card>
                    <n-statistic label="待审核">
                        <n-number-animation :from="0" :to="stats.pending_count" />
                        <template #suffix>
                             <span class="unit">条</span>
                        </template>
                    </n-statistic>
                </n-card>
            </n-grid-item>
            <n-grid-item>
                <n-card>
                    <n-statistic label="累计总数">
                        <n-number-animation :from="0" :to="stats.total_count" />
                    </n-statistic>
                </n-card>
            </n-grid-item>
        </n-grid>

        <n-card title="各阶段分布" style="margin-top: 16px;">
            <div class="progress-list">
                <div v-for="(count, stage) in stats.stage_breakdown" :key="stage" class="progress-item">
                    <div class="label">{{ STAGE_LABELS[stage] ?? stage }}</div>
                    <div class="bar">
                        <n-progress 
                            type="line" 
                            :percentage="stats.total_count > 0 ? Math.round((count / stats.total_count) * 100) : 0" 
                            :height="20"
                            :color="stage === 'review_pending' ? '#f0a020' : undefined"
                        >
                           {{ count }} ({{ stats.total_count > 0 ? ((count / stats.total_count) * 100).toFixed(1) : 0 }}%)
                        </n-progress>
                    </div>
                </div>
            </div>
        </n-card>
    </div>
  </div>
</template>

<style scoped>
.stats-container {
    padding: 0;
}
.header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 16px;
}
.header h2 {
    margin: 0;
}
.refresh-btn {
    border: 1px solid #ccc;
    background: #fff;
    padding: 6px 12px;
    border-radius: 4px;
    cursor: pointer;
}
.loading-wrap {
    padding: 50px;
    display: flex;
    justify-content: center;
}
.progress-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
}
.progress-item {
    display: grid;
    grid-template-columns: 100px 1fr;
    align-items: center;
    gap: 12px;
}
.label {
    text-align: right;
    color: #666;
    font-size: 14px;
}
.unit {
    font-size: 14px;
    margin-left: 4px;
    color: #999;
}
</style>
