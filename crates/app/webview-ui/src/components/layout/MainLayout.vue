<script setup lang="ts">
import { computed, h, ref } from 'vue'
import { useAuth } from '../../composables/useAuth'
import {
  NAvatar,
  NDropdown,
  NIcon,
  NLayout,
  NLayoutContent,
  NLayoutHeader,
  NLayoutSider,
  NMenu,
  NTag,
} from 'naive-ui'
import {
  BarChartOutline as StatsIcon,
  BookOutline as ReviewIcon,
  LogOutOutline as LogoutIcon,
  PersonOutline as UserIcon,
  SparklesOutline as SparklesIcon,
} from '@vicons/ionicons5'

const props = defineProps<{
  activeKey: string
}>()

const emit = defineEmits<{
  (e: 'update:activeKey', key: string): void
}>()

const auth = useAuth()
const collapsed = ref(false)

const menuOptions = [
  {
    label: '稿件审核',
    key: 'review',
    icon: () => h(NIcon, null, { default: () => h(ReviewIcon) }),
  },
  {
    label: '数据统计',
    key: 'stats',
    icon: () => h(NIcon, null, { default: () => h(StatsIcon) }),
  },
]

const userOptions = [
  { label: '退出登录', key: 'logout', icon: () => h(NIcon, null, { default: () => h(LogoutIcon) }) },
]

const viewMeta = computed(() => {
  if (props.activeKey === 'stats') {
    return {
      title: '数据统计',
      subtitle: '查看稿件数量、待审情况和阶段分布',
    }
  }
  return {
    title: '审核工作台',
    subtitle: '查看稿件列表并执行审核操作',
  }
})

const roleLabel = computed(() =>
  auth.me.value?.role === 'global_admin' ? '全局管理员' : '分组管理员',
)

const scopeLabel = computed(() => {
  if (auth.me.value?.role === 'global_admin') return '全部分组'
  const count = auth.me.value?.groups.length ?? 0
  return count > 0 ? `已授权 ${count} 个分组` : '未绑定分组'
})

function handleMenuUpdate(key: string) {
  emit('update:activeKey', key)
}

function handleUserSelect(key: string) {
  if (key === 'logout') {
    auth.logout()
  }
}
</script>

<template>
  <n-layout has-sider position="absolute">
    <n-layout-sider
      collapse-mode="width"
      :collapsed-width="78"
      :width="292"
      :collapsed="collapsed"
      show-trigger
      class="console-sider"
      @collapse="collapsed = true"
      @expand="collapsed = false"
    >
      <div class="brand-shell">
        <div class="brand-mark">
          <n-icon size="20"><SparklesIcon /></n-icon>
        </div>
        <div v-if="!collapsed" class="brand-copy">
          <span class="brand-title">OQQWall</span>
          <span class="brand-subtitle">审核后台</span>
        </div>
      </div>

      <div v-if="!collapsed" class="scope-panel">
        <div class="scope-head">
          <span>账号权限</span>
          <n-tag size="small" round type="success" :bordered="false">{{ roleLabel }}</n-tag>
        </div>
        <strong>{{ auth.me.value?.username ?? '未登录' }}</strong>
        <p>{{ scopeLabel }}</p>
      </div>

      <n-menu
        :collapsed="collapsed"
        :collapsed-width="78"
        :collapsed-icon-size="22"
        :options="menuOptions"
        :value="activeKey"
        class="nav-menu"
        @update:value="handleMenuUpdate"
      />

      <div v-if="!collapsed" class="sider-footer">
        <span>同一账号下可查看稿件、执行审核和查看统计。</span>
      </div>
    </n-layout-sider>

    <n-layout class="main-layout">
      <n-layout-header class="header">
        <div class="header-left">
          <p class="eyebrow">OQQWall</p>
          <div class="header-text">
            <h2>{{ viewMeta.title }}</h2>
            <span>{{ viewMeta.subtitle }}</span>
          </div>
        </div>

        <div class="header-right">
          <div class="header-badge">
            <span class="header-badge-label">当前范围</span>
            <strong>{{ scopeLabel }}</strong>
          </div>
          <n-dropdown :options="userOptions" @select="handleUserSelect">
            <div class="user-profile">
              <n-avatar round size="medium">
                <n-icon><UserIcon /></n-icon>
              </n-avatar>
              <div class="user-copy">
                <span class="username">{{ auth.me.value?.username }}</span>
                <span class="user-role">{{ roleLabel }}</span>
              </div>
            </div>
          </n-dropdown>
        </div>
      </n-layout-header>

      <n-layout-content content-style="padding: 0; background: transparent; min-height: 100%;">
        <div class="content-shell">
          <slot></slot>
        </div>
      </n-layout-content>
    </n-layout>
  </n-layout>
</template>

<style scoped>
.console-sider {
  background: linear-gradient(180deg, rgba(24, 22, 19, 0.96) 0%, rgba(17, 16, 15, 0.96) 100%);
  border-right: 1px solid var(--app-border);
}

.brand-shell {
  height: 88px;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 0 22px;
  margin: 14px 12px 10px;
  border-radius: 22px;
  background: linear-gradient(135deg, rgba(255, 250, 242, 0.08), rgba(255, 250, 242, 0.03));
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.brand-mark {
  width: 42px;
  height: 42px;
  border-radius: 14px;
  display: grid;
  place-items: center;
  color: #f6ecda;
  background:
    radial-gradient(circle at 20% 20%, rgba(255, 255, 255, 0.2), transparent 48%),
    linear-gradient(135deg, #1f8f6a, #355e7b);
  box-shadow: 0 10px 24px rgba(31, 143, 106, 0.28);
}

.brand-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.brand-title {
  font-family: Georgia, "Times New Roman", serif;
  font-size: 24px;
  letter-spacing: 0.08em;
  color: #fff6e9;
}

.brand-subtitle {
  font-size: 11px;
  letter-spacing: 0.2em;
  text-transform: uppercase;
  color: var(--app-text-muted);
}

.scope-panel {
  margin: 0 12px 14px;
  padding: 18px 16px;
  border-radius: 22px;
  background: rgba(255, 250, 242, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.scope-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
  color: var(--app-text-muted);
  font-size: 12px;
}

.scope-panel strong {
  display: block;
  font-size: 18px;
  font-weight: 700;
  color: #fff6e9;
}

.scope-panel p {
  margin: 10px 0 0;
  line-height: 1.6;
  font-size: 13px;
  color: var(--app-text-muted);
}

.nav-menu {
  margin: 0 10px;
}

.sider-footer {
  position: absolute;
  left: 16px;
  right: 16px;
  bottom: 18px;
  padding: 14px 16px;
  border-radius: 18px;
  border: 1px solid rgba(255, 255, 255, 0.05);
  background: rgba(255, 250, 242, 0.04);
  font-size: 12px;
  line-height: 1.7;
  color: var(--app-text-muted);
}

.main-layout {
  background: transparent;
}

.header {
  height: 104px;
  padding: 20px 26px 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: transparent;
}

.header-left {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.eyebrow {
  margin: 0;
  color: rgba(246, 236, 218, 0.54);
  text-transform: uppercase;
  letter-spacing: 0.16em;
  font-size: 11px;
}

.header-text {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.header-text h2 {
  margin: 0;
  font-family: Georgia, "Times New Roman", serif;
  font-size: clamp(28px, 3vw, 40px);
  font-weight: 700;
  letter-spacing: 0.02em;
  color: #fff6e9;
}

.header-text span {
  color: var(--app-text-muted);
  font-size: 14px;
}

.header-right {
  display: flex;
  align-items: center;
  gap: 14px;
}

.header-badge {
  min-width: 180px;
  padding: 12px 14px;
  border-radius: 18px;
  background: rgba(255, 250, 242, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.header-badge-label {
  display: block;
  margin-bottom: 4px;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.12em;
  color: rgba(246, 236, 218, 0.54);
}

.header-badge strong {
  font-size: 14px;
  color: #fff6e9;
}

.user-profile {
  display: flex;
  align-items: center;
  gap: 12px;
  cursor: pointer;
  padding: 10px 14px;
  border-radius: 18px;
  background: rgba(255, 250, 242, 0.07);
  border: 1px solid rgba(255, 255, 255, 0.08);
  transition: transform 0.2s ease, background-color 0.2s ease;
}

.user-profile:hover {
  transform: translateY(-1px);
  background-color: rgba(255, 250, 242, 0.1);
}

.user-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.username {
  font-size: 14px;
  font-weight: 700;
  color: #fff6e9;
}

.user-role {
  font-size: 12px;
  color: rgba(246, 236, 218, 0.62);
}

.content-shell {
  min-height: calc(100vh - 104px);
  padding: 8px 26px 28px;
}

@media (max-width: 960px) {
  .header {
    height: auto;
    padding-bottom: 12px;
    align-items: flex-start;
    flex-direction: column;
    gap: 16px;
  }

  .header-right {
    width: 100%;
    justify-content: space-between;
  }

  .header-badge {
    min-width: 0;
    flex: 1;
  }

  .content-shell {
    min-height: auto;
    padding: 8px 16px 20px;
  }
}

@media (max-width: 640px) {
  .header-right {
    flex-direction: column;
    align-items: stretch;
  }

  .user-profile {
    justify-content: center;
  }
}
</style>
