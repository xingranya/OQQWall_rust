<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'

type Role = 'global_admin' | 'group_admin'
type Stage =
  | 'drafted'
  | 'render_requested'
  | 'rendered'
  | 'review_pending'
  | 'reviewed'
  | 'scheduled'
  | 'sending'
  | 'sent'
  | 'rejected'
  | 'skipped'
  | 'manual'
  | 'failed'

interface MeResponse {
  username: string
  role: Role
  groups: string[]
  expires_at: number
}

interface PostItem {
  post_id: string
  review_id: string | null
  group_id: string
  stage: Stage
  external_code: number | null
  internal_code: number | null
  sender_id: string | null
  created_at_ms: number
  last_error: string | null
}

interface PostDetail {
  post_id: string
  review_id: string | null
  review_code: number | null
  group_id: string
  stage: Stage
  external_code: number | null
  sender_id: string | null
  session_id: string
  created_at_ms: number
  is_anonymous: boolean
  is_safe: boolean
  blocks: Array<
    | { kind: 'text'; text: string }
    | {
        kind: 'attachment'
        media_kind: string
        reference_type: 'blob_id' | 'remote_url'
        reference: string
        size_bytes: number | null
      }
  >
  render_png_blob_id: string | null
  last_error: string | null
}

interface ApiErrorBody {
  error?: {
    message?: string
  }
}

const loading = ref(false)
const loginLoading = ref(false)
const loginForm = reactive({ username: '', password: '' })
const authed = ref(false)
const me = ref<MeResponse | null>(null)
const notice = ref('')
const stage = ref<Stage>('review_pending')
const posts = ref<PostItem[]>([])
const selectedReviewIds = ref<string[]>([])
const detail = ref<PostDetail | null>(null)
const detailOpen = ref(false)
const detailLoading = ref(false)
const actionLoading = ref(false)

const actionForm = reactive({
  action: 'approve',
  delay_ms: '180000',
  comment: '',
  text: '',
  quick_reply_key: '',
  target_review_code: '',
})

const actionOptions = [
  'approve',
  'reject',
  'delete',
  'defer',
  'skip',
  'immediate',
  'refresh',
  'rerender',
  'select_all',
  'toggle_anonymous',
  'expand_audit',
  'show',
  'comment',
  'reply',
  'blacklist',
  'quick_reply',
  'merge',
]

const stages: Stage[] = [
  'review_pending',
  'reviewed',
  'scheduled',
  'sending',
  'failed',
  'manual',
  'sent',
  'rejected',
  'skipped',
  'drafted',
  'render_requested',
  'rendered',
]

const pendingCount = computed(() => posts.value.filter((item) => item.stage === 'review_pending').length)

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers ?? {}),
    },
    ...init,
  })
  if (!res.ok) {
    let message = `请求失败: ${res.status}`
    try {
      const body = (await res.json()) as ApiErrorBody
      if (body?.error?.message) message = body.error.message
    } catch {
      // ignore
    }
    throw new Error(message)
  }
  if (res.status === 204) return {} as T
  return (await res.json()) as T
}

function resetActionForm() {
  actionForm.comment = ''
  actionForm.text = ''
  actionForm.quick_reply_key = ''
  actionForm.target_review_code = ''
}

async function checkSession() {
  try {
    const result = await api<MeResponse>('/auth/me', { method: 'GET' })
    authed.value = true
    me.value = result
  } catch {
    authed.value = false
    me.value = null
  }
}

async function login() {
  if (!loginForm.username.trim() || !loginForm.password.trim()) {
    notice.value = '请输入用户名和密码'
    return
  }
  loginLoading.value = true
  notice.value = ''
  try {
    await api('/auth/login', {
      method: 'POST',
      body: JSON.stringify({
        username: loginForm.username.trim(),
        password: loginForm.password,
      }),
    })
    authed.value = true
    loginForm.password = ''
    await checkSession()
    await loadPosts()
  } catch (err) {
    notice.value = (err as Error).message
  } finally {
    loginLoading.value = false
  }
}

async function logout() {
  try {
    await api('/auth/logout', { method: 'POST' })
  } finally {
    authed.value = false
    me.value = null
    posts.value = []
    selectedReviewIds.value = []
    detail.value = null
  }
}

async function loadPosts() {
  loading.value = true
  notice.value = ''
  try {
    const result = await api<{ items: PostItem[] }>('/api/posts?stage=' + stage.value + '&limit=200')
    posts.value = result.items
    const reviewSet = new Set(result.items.map((item) => item.review_id).filter(Boolean) as string[])
    selectedReviewIds.value = selectedReviewIds.value.filter((id) => reviewSet.has(id))
  } catch (err) {
    notice.value = (err as Error).message
  } finally {
    loading.value = false
  }
}

function toggleSelectAll() {
  const all = posts.value
    .map((item) => item.review_id)
    .filter(Boolean) as string[]
  if (selectedReviewIds.value.length === all.length) {
    selectedReviewIds.value = []
    return
  }
  selectedReviewIds.value = all
}

function toggleOneSelection(reviewId: string, checked: boolean) {
  if (checked) {
    if (!selectedReviewIds.value.includes(reviewId)) {
      selectedReviewIds.value = [...selectedReviewIds.value, reviewId]
    }
    return
  }
  selectedReviewIds.value = selectedReviewIds.value.filter((id) => id !== reviewId)
}

async function openDetail(postId: string) {
  detailLoading.value = true
  detailOpen.value = true
  notice.value = ''
  try {
    detail.value = await api<PostDetail>('/api/posts/' + postId)
  } catch (err) {
    notice.value = (err as Error).message
    detail.value = null
  } finally {
    detailLoading.value = false
  }
}

function payloadFromActionForm() {
  const payload: Record<string, unknown> = { action: actionForm.action }
  const delay = Number(actionForm.delay_ms)
  const code = Number(actionForm.target_review_code)
  if (!Number.isNaN(delay)) payload.delay_ms = delay
  if (actionForm.comment.trim()) payload.comment = actionForm.comment.trim()
  if (actionForm.text.trim()) payload.text = actionForm.text.trim()
  if (actionForm.quick_reply_key.trim()) payload.quick_reply_key = actionForm.quick_reply_key.trim()
  if (!Number.isNaN(code) && code > 0) payload.target_review_code = code
  return payload
}

async function runSingleAction(reviewId: string) {
  actionLoading.value = true
  notice.value = ''
  try {
    await api('/api/reviews/' + reviewId + '/decision', {
      method: 'POST',
      body: JSON.stringify(payloadFromActionForm()),
    })
    await loadPosts()
    if (detail.value?.post_id) await openDetail(detail.value.post_id)
    notice.value = `已执行 ${actionForm.action}`
    resetActionForm()
  } catch (err) {
    notice.value = (err as Error).message
  } finally {
    actionLoading.value = false
  }
}

async function runBatchAction() {
  if (selectedReviewIds.value.length === 0) {
    notice.value = '请先选择至少一条待处理项'
    return
  }
  actionLoading.value = true
  notice.value = ''
  try {
    const result = await api<{ accepted: number; failed: { review_id: string; reason: string }[] }>(
      '/api/reviews/batch',
      {
        method: 'POST',
        body: JSON.stringify({
          review_ids: selectedReviewIds.value,
          ...payloadFromActionForm(),
        }),
      },
    )
    await loadPosts()
    const failureText = result.failed.length
      ? `，失败 ${result.failed.length} 条`
      : ''
    notice.value = `批量执行完成：成功 ${result.accepted} 条${failureText}`
    resetActionForm()
  } catch (err) {
    notice.value = (err as Error).message
  } finally {
    actionLoading.value = false
  }
}

function renderImageUrl(blockRef: { reference_type: 'blob_id' | 'remote_url'; reference: string }) {
  if (blockRef.reference_type === 'blob_id') return '/api/blobs/' + blockRef.reference
  return blockRef.reference
}

function formatTime(ms: number) {
  return new Date(ms).toLocaleString('zh-CN')
}

onMounted(async () => {
  await checkSession()
  if (authed.value) await loadPosts()
})
</script>

<template>
  <div class="page">
    <header class="topbar">
      <div>
        <h1>OQQWall Web Review</h1>
        <p v-if="me">{{ me.username }} · {{ me.role }} · 已授权组 {{ me.groups.length || 'ALL' }}</p>
      </div>
      <div class="top-actions" v-if="authed">
        <button class="btn" @click="loadPosts" :disabled="loading">刷新</button>
        <button class="btn danger" @click="logout">退出</button>
      </div>
    </header>

    <main v-if="!authed" class="login-wrap">
      <section class="card login-card">
        <h2>登录</h2>
        <label>用户名</label>
        <input v-model="loginForm.username" type="text" autocomplete="username" />
        <label>密码</label>
        <input v-model="loginForm.password" type="password" autocomplete="current-password" @keyup.enter="login" />
        <button class="btn primary" @click="login" :disabled="loginLoading">{{ loginLoading ? '登录中...' : '登录' }}</button>
        <p class="notice" v-if="notice">{{ notice }}</p>
      </section>
    </main>

    <main v-else class="workspace">
      <section class="card controls">
        <div class="stage-row">
          <label>阶段</label>
          <select v-model="stage" @change="loadPosts">
            <option v-for="s in stages" :value="s" :key="s">{{ s }}</option>
          </select>
          <span class="muted">当前 {{ posts.length }} 条，待审核 {{ pendingCount }} 条</span>
        </div>

        <div class="action-grid">
          <div>
            <label>动作</label>
            <select v-model="actionForm.action">
              <option v-for="action in actionOptions" :value="action" :key="action">{{ action }}</option>
            </select>
          </div>
          <div>
            <label>delay_ms</label>
            <input v-model="actionForm.delay_ms" type="number" />
          </div>
          <div>
            <label>comment</label>
            <input v-model="actionForm.comment" type="text" />
          </div>
          <div>
            <label>text</label>
            <input v-model="actionForm.text" type="text" />
          </div>
          <div>
            <label>quick_reply_key</label>
            <input v-model="actionForm.quick_reply_key" type="text" />
          </div>
          <div>
            <label>target_review_code</label>
            <input v-model="actionForm.target_review_code" type="number" />
          </div>
        </div>

        <div class="stage-row">
          <button class="btn" @click="toggleSelectAll">{{ selectedReviewIds.length === posts.filter(p => p.review_id).length ? '取消全选' : '全选可审核项' }}</button>
          <button class="btn primary" @click="runBatchAction" :disabled="actionLoading">批量执行（{{ selectedReviewIds.length }}）</button>
        </div>
      </section>

      <p class="notice" v-if="notice">{{ notice }}</p>

      <section class="grid" v-if="!loading">
        <article class="card item" v-for="post in posts" :key="post.post_id">
          <div class="item-head">
            <div>
              <strong>#{{ post.internal_code ?? '-' }}</strong>
              <span class="muted">{{ post.group_id }} · {{ post.stage }}</span>
            </div>
            <input
              v-if="post.review_id"
              type="checkbox"
              :checked="selectedReviewIds.includes(post.review_id)"
              @change="($event) => toggleOneSelection(post.review_id as string, ($event.target as HTMLInputElement).checked)"
            />
          </div>
          <p class="muted">投稿人：{{ post.sender_id ?? '未知' }}</p>
          <p class="muted">创建时间：{{ formatTime(post.created_at_ms) }}</p>
          <p class="err" v-if="post.last_error">{{ post.last_error }}</p>
          <div class="item-actions">
            <button class="btn" @click="openDetail(post.post_id)">详情</button>
            <button class="btn primary" v-if="post.review_id" @click="runSingleAction(post.review_id)">执行动作</button>
          </div>
        </article>
      </section>
      <section v-else class="card">加载中...</section>
    </main>

    <aside class="drawer" v-if="detailOpen">
      <section class="card drawer-card">
        <header class="item-head">
          <h3>稿件详情</h3>
          <button class="btn" @click="detailOpen = false">关闭</button>
        </header>
        <div v-if="detailLoading">加载中...</div>
        <div v-else-if="detail">
          <p class="muted">post_id: {{ detail.post_id }}</p>
          <p class="muted">review_id: {{ detail.review_id }}</p>
          <p class="muted">group: {{ detail.group_id }} · {{ detail.stage }}</p>
          <p class="muted">匿名: {{ detail.is_anonymous }} · 安全: {{ detail.is_safe }}</p>
          <img v-if="detail.render_png_blob_id" :src="'/api/blobs/' + detail.render_png_blob_id" class="preview" alt="render preview" />
          <div class="blocks">
            <div class="block" v-for="(block, idx) in detail.blocks" :key="idx">
              <p v-if="block.kind === 'text'">{{ block.text }}</p>
              <div v-else>
                <p class="muted">{{ block.media_kind }} · {{ block.reference_type }}</p>
                <img
                  v-if="block.media_kind === 'image'"
                  :src="renderImageUrl(block)"
                  class="thumb"
                  alt="attachment"
                />
                <a v-else :href="renderImageUrl(block)" target="_blank" rel="noreferrer">打开附件</a>
              </div>
            </div>
          </div>
        </div>
      </section>
    </aside>
  </div>
</template>

<style scoped>
:global(body) {
  margin: 0;
  font-family: 'IBM Plex Sans', 'Noto Sans SC', sans-serif;
  background: radial-gradient(circle at 20% 20%, #fef6e4 0%, #f4f4f8 45%, #eef6ff 100%);
}

.page {
  min-height: 100vh;
  padding: 16px;
  color: #14213d;
}

.topbar {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
  margin-bottom: 16px;
}

.topbar h1 {
  margin: 0;
  font-size: 28px;
  letter-spacing: 0.4px;
}

.topbar p {
  margin: 6px 0 0;
}

.workspace,
.login-wrap {
  display: grid;
  gap: 16px;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 12px;
}

.card {
  background: rgba(255, 255, 255, 0.85);
  border: 1px solid rgba(20, 33, 61, 0.15);
  backdrop-filter: blur(8px);
  border-radius: 16px;
  padding: 14px;
  box-shadow: 0 10px 20px rgba(20, 33, 61, 0.06);
}

.login-card {
  max-width: 420px;
  margin: 10vh auto 0;
  display: grid;
  gap: 10px;
}

label {
  font-size: 12px;
  color: #555;
}

input,
select {
  width: 100%;
  border: 1px solid rgba(20, 33, 61, 0.25);
  border-radius: 10px;
  min-height: 36px;
  padding: 0 10px;
  background: #fff;
  color: #111;
}

.controls {
  display: grid;
  gap: 12px;
}

.stage-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
}

.action-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: 10px;
}

.item {
  display: grid;
  gap: 8px;
}

.item-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}

.item-actions,
.top-actions {
  display: flex;
  gap: 8px;
}

.btn {
  min-height: 36px;
  border: 1px solid rgba(20, 33, 61, 0.3);
  border-radius: 999px;
  padding: 0 14px;
  background: rgba(255, 255, 255, 0.95);
  color: #14213d;
  cursor: pointer;
}

.btn.primary {
  background: #1d3557;
  color: #fff;
  border-color: #1d3557;
}

.btn.danger {
  background: #e63946;
  color: #fff;
  border-color: #e63946;
}

.btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.muted {
  color: #516079;
  font-size: 12px;
}

.notice {
  margin: 0;
  color: #9b2226;
  font-weight: 600;
}

.err {
  color: #c1121f;
  font-size: 12px;
}

.drawer {
  position: fixed;
  right: 0;
  top: 0;
  width: min(680px, 100vw);
  height: 100vh;
  padding: 16px;
  background: rgba(17, 20, 27, 0.28);
}

.drawer-card {
  height: calc(100vh - 32px);
  overflow: auto;
}

.preview {
  width: 100%;
  border-radius: 12px;
  border: 1px solid rgba(20, 33, 61, 0.16);
}

.blocks {
  display: grid;
  gap: 8px;
}

.block {
  border: 1px solid rgba(20, 33, 61, 0.12);
  border-radius: 10px;
  padding: 8px;
}

.thumb {
  width: 120px;
  border-radius: 8px;
  border: 1px solid rgba(20, 33, 61, 0.16);
}
</style>
