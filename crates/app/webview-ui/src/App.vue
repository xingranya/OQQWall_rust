<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { NConfigProvider, NMessageProvider, NGlobalStyle, zhCN, dateZhCN } from 'naive-ui'
import MainLayout from './components/layout/MainLayout.vue'
import LoginForm from './components/business/LoginForm.vue'
import ReviewWorkspace from './components/business/ReviewWorkspace.vue'
import StatsDashboard from './components/business/StatsDashboard.vue'
import { useAuth } from './composables/useAuth'

const auth = useAuth()
const currentView = ref('review')

onMounted(() => {
  auth.checkSession()
})
</script>

<template>
  <n-config-provider :locale="zhCN" :date-locale="dateZhCN">
    <n-global-style />
    <n-message-provider>
      <div v-if="!auth.authed.value">
        <LoginForm />
      </div>
      <MainLayout v-else v-model:activeKey="currentView">
        <KeepAlive>
            <component :is="currentView === 'stats' ? StatsDashboard : ReviewWorkspace" />
        </KeepAlive>
      </MainLayout>
    </n-message-provider>
  </n-config-provider>
</template>

<style>
body {
    margin: 0;
    padding: 0;
    font-family: v-sans, system-ui, -apple-system, sans-serif;
    background-color: #f0f2f5;
}
</style>
