<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import {
  NCard,
  NGrid,
  NGridItem,
  NNumberAnimation,
  NProgress,
  NSpin,
  NStatistic,
  NTag,
  useMessage,
} from 'naive-ui'
import { api } from '../../api/client'
import { STAGE_LABELS, type StatsResponse } from '../../api/types'

const message = useMessage()
const loading = ref(false)
const stats = ref<StatsResponse | null>(null)

const stageEntries = computed(() => {
  if (!stats.value) return []
  return Object.entries(stats.value.stage_breakdown)
    .map(([stage, count]) => ({
      stage,
      count,
      percentage: stats.value && stats.value.total_count > 0
        ? Number(((count / stats.value.total_count) * 100).toFixed(1))
        : 0,
    }))
    .sort((a, b) => b.count - a.count)
})

const leadStage = computed(() => stageEntries.value[0] ?? null)

const cardMetrics = computed(() => {
  if (!stats.value) return []
  return [
    {
      label: '今日投稿',
      value: stats.value.today_count,
      hint: '自然日内新增入库稿件',
    },
    {
      label: '待审核',
      value: stats.value.pending_count,
      hint: '仍需人工确认的稿件',
    },
    {
      label: '累计总数',
      value: stats.value.total_count,
      hint: '当前状态视图中的总稿件数',
    },
  ]
})

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
    <section class="stats-hero">
      <div>
        <span class="hero-kicker">数据统计</span>
        <h1>查看当前稿件总量、待审数量和阶段分布。</h1>
        <p>用于快速了解审核积压和整体处理进度。</p>
      </div>
      <div class="hero-side">
        <n-tag v-if="leadStage" round :bordered="false" type="warning">
          占比最高：{{ STAGE_LABELS[leadStage.stage] ?? leadStage.stage }} {{ leadStage.percentage }}%
        </n-tag>
        <button class="refresh-btn" @click="loadStats">刷新数据</button>
      </div>
    </section>

    <div v-if="loading && !stats" class="loading-wrap">
      <n-spin size="large" />
    </div>

    <div v-else-if="stats" class="dashboard">
      <n-grid x-gap="14" y-gap="14" cols="1 s:2 m:3" responsive="screen">
        <n-grid-item v-for="metric in cardMetrics" :key="metric.label">
          <n-card class="stat-card" :bordered="false">
            <span class="stat-label">{{ metric.label }}</span>
            <n-statistic>
              <n-number-animation :from="0" :to="metric.value" />
            </n-statistic>
            <p>{{ metric.hint }}</p>
          </n-card>
        </n-grid-item>
      </n-grid>

      <section class="distribution-panel">
        <div class="panel-head">
          <div>
            <span class="panel-kicker">阶段结构</span>
            <h2>各阶段分布</h2>
          </div>
          <span class="panel-note">总稿件 {{ stats.total_count }} 条</span>
        </div>

        <div class="progress-list">
          <div v-for="entry in stageEntries" :key="entry.stage" class="progress-item">
            <div class="label-block">
              <strong>{{ STAGE_LABELS[entry.stage] ?? entry.stage }}</strong>
              <span>{{ entry.count }} 条</span>
            </div>
            <div class="bar">
              <n-progress
                type="line"
                :percentage="entry.percentage"
                :height="18"
                :color="entry.stage === 'review_pending' ? '#c87a2a' : '#1f8f6a'"
              >
                {{ entry.percentage }}%
              </n-progress>
            </div>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.stats-container {
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.stats-hero {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 24px;
  border-radius: 28px;
  background: linear-gradient(180deg, rgba(255, 250, 242, 0.08), rgba(255, 250, 242, 0.04));
  border: 1px solid rgba(255, 255, 255, 0.08);
  backdrop-filter: blur(14px);
}

.hero-kicker,
.panel-kicker {
  display: inline-block;
  margin-bottom: 10px;
  font-size: 11px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: rgba(246, 236, 218, 0.6);
}

.stats-hero h1 {
  margin: 0;
  max-width: 16ch;
  font-family: Georgia, "Times New Roman", serif;
  font-size: clamp(28px, 3.4vw, 42px);
  line-height: 1.1;
  color: #fff6e9;
}

.stats-hero p {
  max-width: 38rem;
  margin: 14px 0 0;
  color: rgba(246, 236, 218, 0.72);
  line-height: 1.7;
}

.hero-side {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 12px;
}

.refresh-btn {
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: rgba(255, 250, 242, 0.08);
  color: #fff3e1;
  padding: 8px 14px;
  border-radius: 999px;
  cursor: pointer;
}

.loading-wrap {
  padding: 50px;
  display: flex;
  justify-content: center;
  border-radius: 28px;
  background: rgba(255, 250, 242, 0.06);
  border: 1px solid rgba(255, 255, 255, 0.08);
}

.dashboard {
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.stat-card {
  height: 100%;
  border-radius: 24px;
  background: rgba(255, 248, 238, 0.94);
  box-shadow: 0 18px 34px rgba(12, 10, 8, 0.12);
  transition: transform 0.18s ease, box-shadow 0.18s ease;
}

.stat-card:hover {
  transform: translateY(-3px);
  box-shadow: 0 24px 40px rgba(12, 10, 8, 0.16);
}

.stat-label {
  display: block;
  margin-bottom: 10px;
  color: rgba(38, 29, 23, 0.62);
  font-size: 12px;
  letter-spacing: 0.08em;
}

.stat-card p {
  margin: 10px 0 0;
  color: rgba(38, 29, 23, 0.58);
  line-height: 1.7;
  font-size: 13px;
}

.distribution-panel {
  padding: 24px;
  border-radius: 28px;
  background: rgba(255, 248, 238, 0.94);
  box-shadow: 0 18px 34px rgba(12, 10, 8, 0.12);
}

.panel-head {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 18px;
}

.panel-head h2 {
  margin: 0;
  font-size: 28px;
  color: #261d17;
}

.panel-note {
  color: rgba(38, 29, 23, 0.58);
  font-size: 13px;
}

.progress-list {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.progress-item {
  display: grid;
  grid-template-columns: 170px 1fr;
  align-items: center;
  gap: 14px;
}

.label-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.label-block strong {
  color: #261d17;
}

.label-block span {
  font-size: 12px;
  color: rgba(38, 29, 23, 0.58);
}

@media (max-width: 760px) {
  .stats-hero,
  .panel-head,
  .hero-side {
    flex-direction: column;
    align-items: flex-start;
  }

  .progress-item {
    grid-template-columns: 1fr;
  }
}
</style>
