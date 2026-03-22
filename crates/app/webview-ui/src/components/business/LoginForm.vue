<script setup lang="ts">
import { computed, reactive } from 'vue'
import {
  NButton,
  NCard,
  NForm,
  NFormItem,
  NIcon,
  NInput,
  NTag,
  useMessage,
} from 'naive-ui'
import {
  KeyOutline as KeyIcon,
  LockClosedOutline as LockIcon,
  PersonCircleOutline as PersonIcon,
  SparklesOutline as SparklesIcon,
} from '@vicons/ionicons5'
import { useAuth } from '../../composables/useAuth'

const auth = useAuth()
const message = useMessage()

const form = reactive({
  username: '',
  password: '',
})

const greeting = computed(() =>
  auth.loginLoading.value ? '正在校验登录信息' : '登录审核后台',
)

async function handleLogin() {
  if (!form.username || !form.password) {
    message.warning('请输入用户名和密码')
    return
  }
  try {
    await auth.login(form.username, form.password)
    message.success('登录成功')
  } catch (e) {
    message.error((e as Error).message)
  }
}
</script>

<template>
  <div class="login-container">
    <div class="login-hero">
      <div class="hero-copy">
        <div class="hero-badge">
          <n-icon size="16"><SparklesIcon /></n-icon>
          <span>OQQWall 审核后台</span>
        </div>
        <h1>登录后可查看稿件、处理审核并查看统计。</h1>
        <p>适用于管理员日常值班和集中处理。</p>
        <div class="hero-points">
          <div class="point-card">
            <strong>稿件列表</strong>
            <span>按状态筛选并进入详情处理。</span>
          </div>
          <div class="point-card">
            <strong>权限控制</strong>
            <span>账号范围和登录状态由系统统一校验。</span>
          </div>
        </div>
      </div>

      <n-card class="login-card" size="huge" :bordered="false">
        <div class="card-head">
          <n-tag size="small" round type="success" :bordered="false">管理员登录</n-tag>
          <h2>{{ greeting }}</h2>
          <p>使用已配置的管理员账号登录。</p>
        </div>

        <n-form>
          <n-form-item label="用户名">
            <n-input v-model:value="form.username" placeholder="请输入管理员用户名" @keyup.enter="handleLogin">
              <template #prefix>
                <n-icon><PersonIcon /></n-icon>
              </template>
            </n-input>
          </n-form-item>

          <n-form-item label="密码">
            <n-input
              v-model:value="form.password"
              type="password"
              show-password-on="click"
              placeholder="请输入登录密码"
              @keyup.enter="handleLogin"
            >
              <template #prefix>
                <n-icon><LockIcon /></n-icon>
              </template>
            </n-input>
          </n-form-item>

          <n-button type="primary" block @click="handleLogin" :loading="auth.loginLoading.value" size="large">
            <template #icon>
              <n-icon><KeyIcon /></n-icon>
            </template>
            进入审核台
          </n-button>
        </n-form>

        <div class="card-foot">
          <span>登录后将使用站内会话维持身份。</span>
          <span>如无法登录，请联系系统管理员检查账号权限。</span>
        </div>
      </n-card>
    </div>
  </div>
</template>

<style scoped>
.login-container {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 32px;
}

.login-hero {
  width: min(1180px, 100%);
  display: grid;
  grid-template-columns: minmax(0, 1.18fr) minmax(340px, 420px);
  gap: 28px;
  align-items: stretch;
}

.hero-copy,
.login-card {
  position: relative;
  overflow: hidden;
  border-radius: 28px;
  box-shadow: var(--app-shadow);
}

.hero-copy {
  padding: 44px;
  background:
    radial-gradient(circle at top right, rgba(31, 143, 106, 0.22), transparent 34%),
    linear-gradient(160deg, rgba(255, 250, 242, 0.1), rgba(255, 250, 242, 0.04));
  border: 1px solid rgba(255, 255, 255, 0.08);
  backdrop-filter: blur(16px);
}

.hero-badge {
  width: fit-content;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 24px;
  padding: 10px 14px;
  border-radius: 999px;
  background: rgba(255, 250, 242, 0.08);
  border: 1px solid rgba(255, 255, 255, 0.08);
  color: #f8efdf;
  letter-spacing: 0.06em;
}

.hero-copy h1 {
  margin: 0;
  max-width: 9em;
  font-family: Georgia, "Times New Roman", serif;
  font-size: clamp(34px, 4.2vw, 54px);
  line-height: 1.08;
  color: #fff6e9;
}

.hero-copy p {
  max-width: 34rem;
  margin: 18px 0 0;
  font-size: 15px;
  line-height: 1.8;
  color: rgba(246, 236, 218, 0.78);
}

.hero-points {
  margin-top: 28px;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
}

.point-card {
  padding: 18px;
  border-radius: 22px;
  background: rgba(255, 250, 242, 0.06);
  border: 1px solid rgba(255, 255, 255, 0.08);
}

.point-card strong {
  display: block;
  margin-bottom: 8px;
  color: #fff3e1;
  font-size: 16px;
}

.point-card span {
  display: block;
  line-height: 1.7;
  color: rgba(246, 236, 218, 0.72);
  font-size: 13px;
}

.login-card {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  background: rgba(255, 248, 238, 0.92);
  color: #2a211b;
}

.card-head h2 {
  margin: 16px 0 10px;
  font-family: Georgia, "Times New Roman", serif;
  font-size: 32px;
  line-height: 1.12;
  color: #221a15;
}

.card-head p {
  margin: 0 0 28px;
  color: rgba(42, 33, 27, 0.72);
  line-height: 1.7;
}

.card-foot {
  display: grid;
  gap: 6px;
  margin-top: 20px;
  padding-top: 18px;
  border-top: 1px solid rgba(34, 26, 21, 0.08);
  color: rgba(42, 33, 27, 0.56);
  font-size: 12px;
  line-height: 1.7;
}

@media (max-width: 960px) {
  .login-container {
    padding: 18px;
  }

  .login-hero {
    grid-template-columns: 1fr;
  }

  .hero-copy {
    padding: 32px 24px;
  }

  .hero-copy h1 {
    max-width: none;
  }

  .hero-points {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 640px) {
  .login-card :deep(.n-card__content) {
    padding-left: 20px;
    padding-right: 20px;
  }

  .hero-copy {
    padding: 28px 20px;
  }
}
</style>
