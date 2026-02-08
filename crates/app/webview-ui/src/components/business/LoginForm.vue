<script setup lang="ts">
import { reactive, ref } from 'vue'
import { useAuth } from '../../composables/useAuth'
import { NCard, NInput, NButton, NForm, NFormItem, useMessage } from 'naive-ui'

const auth = useAuth()
const message = useMessage()

const form = reactive({
  username: '',
  password: '',
})

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
    <n-card title="OQQWall 审核后台" class="login-card" size="huge" :bordered="false">
      <p class="sub-text">使用管理员账号登录以继续</p>
      <n-form>
        <n-form-item label="用户名">
          <n-input v-model:value="form.username" placeholder="User" @keyup.enter="handleLogin" />
        </n-form-item>
        <n-form-item label="密码">
          <n-input
            v-model:value="form.password"
            type="password"
            show-password-on="click"
            placeholder="Password"
            @keyup.enter="handleLogin"
          />
        </n-form-item>
        <n-button type="primary" block @click="handleLogin" :loading="auth.loginLoading.value" size="large">
          登录
        </n-button>
      </n-form>
    </n-card>
  </div>
</template>

<style scoped>
.login-container {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #f5f7fa 0%, #c3cfe2 100%);
}
.login-card {
  width: 100%;
  max-width: 420px;
  border-radius: 16px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.08);
}
.sub-text {
  margin-top: -10px;
  margin-bottom: 24px;
  color: #666;
}
</style>
