<script setup lang="ts">
import { KeepAlive, computed, onMounted, ref } from 'vue'
import {
  NConfigProvider,
  NGlobalStyle,
  NMessageProvider,
  dateZhCN,
  zhCN,
} from 'naive-ui'
import MainLayout from './components/layout/MainLayout.vue'
import LoginForm from './components/business/LoginForm.vue'
import ReviewWorkspace from './components/business/ReviewWorkspace.vue'
import StatsDashboard from './components/business/StatsDashboard.vue'
import { useAuth } from './composables/useAuth'

const auth = useAuth()
const currentView = ref('review')

const themeOverrides = computed(() => ({
  common: {
    primaryColor: '#1f8f6a',
    primaryColorHover: '#167b5b',
    primaryColorPressed: '#0f5f47',
    primaryColorSuppl: '#2aa77f',
    infoColor: '#355e7b',
    successColor: '#1f8f6a',
    warningColor: '#c87a2a',
    errorColor: '#b84d3a',
    borderRadius: '18px',
    borderRadiusSmall: '12px',
    cardColor: '#fffdfa',
    bodyColor: '#f6f1e8',
    modalColor: '#fffaf2',
    textColorBase: '#261d17',
    textColor1: '#261d17',
    textColor2: 'rgba(38, 29, 23, 0.78)',
    textColor3: 'rgba(38, 29, 23, 0.58)',
    placeholderColor: 'rgba(38, 29, 23, 0.38)',
    borderColor: 'rgba(75, 62, 53, 0.14)',
    dividerColor: 'rgba(75, 62, 53, 0.12)',
    fontFamily: '"Lato", "PingFang SC", "Microsoft YaHei", sans-serif',
    fontFamilyMono: '"Fira Code", "Cascadia Code", monospace',
  },
}))

onMounted(() => {
  auth.checkSession()
})
</script>

<template>
  <n-config-provider :locale="zhCN" :date-locale="dateZhCN" :theme-overrides="themeOverrides">
    <n-global-style />
    <n-message-provider>
      <div class="app-shell">
        <div class="ambient ambient-left"></div>
        <div class="ambient ambient-right"></div>
        <div class="grain-layer"></div>
        <div class="app-stage">
          <LoginForm v-if="!auth.authed.value" />
          <MainLayout v-else v-model:activeKey="currentView">
            <KeepAlive>
              <component :is="currentView === 'stats' ? StatsDashboard : ReviewWorkspace" />
            </KeepAlive>
          </MainLayout>
        </div>
      </div>
    </n-message-provider>
  </n-config-provider>
</template>

<style>
:root {
  color-scheme: light;
  --app-bg: #f3ede3;
  --app-page: #f6f1e8;
  --app-page-strong: #f9f6ef;
  --app-panel: rgba(33, 29, 25, 0.94);
  --app-panel-strong: rgba(24, 22, 19, 0.98);
  --app-card: rgba(255, 250, 242, 0.96);
  --app-card-soft: rgba(246, 239, 228, 0.9);
  --app-border: rgba(220, 199, 166, 0.18);
  --app-border-strong: rgba(75, 62, 53, 0.12);
  --app-text: #261d17;
  --app-text-muted: rgba(38, 29, 23, 0.62);
  --app-text-on-dark: #f6ecda;
  --app-text-on-dark-muted: rgba(246, 236, 218, 0.66);
  --app-accent: #1f8f6a;
  --app-warning: #c87a2a;
  --app-danger: #b84d3a;
  --app-shadow: 0 26px 70px rgba(22, 16, 11, 0.18);
  --app-shadow-soft: 0 18px 40px rgba(22, 16, 11, 0.1);
}

* {
  box-sizing: border-box;
}

html,
body,
#app {
  min-height: 100%;
}

body {
  margin: 0;
  padding: 0;
  font-family: "Lato", "PingFang SC", "Microsoft YaHei", sans-serif;
  background:
    radial-gradient(circle at top left, rgba(31, 143, 106, 0.12), transparent 24%),
    radial-gradient(circle at 80% 10%, rgba(200, 122, 42, 0.1), transparent 22%),
    linear-gradient(180deg, #f7f3eb 0%, #f2ebdf 100%);
  color: var(--app-text);
}

a {
  color: inherit;
}

.app-shell {
  position: relative;
  min-height: 100vh;
  overflow: hidden;
}

.app-stage {
  position: relative;
  z-index: 2;
  min-height: 100vh;
}

.ambient {
  position: fixed;
  border-radius: 999px;
  filter: blur(24px);
  opacity: 0.7;
  pointer-events: none;
  z-index: 0;
}

.ambient-left {
  top: 6vh;
  left: -10vw;
  width: 28vw;
  height: 28vw;
  background: radial-gradient(circle, rgba(31, 143, 106, 0.16), transparent 65%);
}

.ambient-right {
  right: -8vw;
  bottom: 10vh;
  width: 32vw;
  height: 32vw;
  background: radial-gradient(circle, rgba(200, 122, 42, 0.12), transparent 64%);
}

.grain-layer {
  position: fixed;
  inset: 0;
  z-index: 1;
  pointer-events: none;
  opacity: 0.04;
  background-image:
    linear-gradient(rgba(255, 255, 255, 0.4) 1px, transparent 1px),
    linear-gradient(90deg, rgba(255, 255, 255, 0.4) 1px, transparent 1px);
  background-size: 3px 3px, 3px 3px;
  mix-blend-mode: soft-light;
}
</style>
