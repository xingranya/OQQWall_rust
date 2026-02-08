<script setup lang="ts">
import { h, ref } from 'vue'
import { useAuth } from '../../composables/useAuth'
import { 
  NLayout, NLayoutSider, NLayoutHeader, NLayoutContent, 
  NMenu, NDropdown, NAvatar, NText, NIcon 
} from 'naive-ui'
import { 
  BookOutline as ReviewIcon, 
  LogOutOutline as LogoutIcon,
  PersonOutline as UserIcon,
  BarChartOutline as StatsIcon
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
    icon: () => h(NIcon, null, { default: () => h(ReviewIcon) })
  },
  {
    label: '数据统计',
    key: 'stats',
    icon: () => h(NIcon, null, { default: () => h(StatsIcon) })
  }
]

const userOptions = [
  { label: '退出登录', key: 'logout', icon: () => h(NIcon, null, { default: () => h(LogoutIcon) }) }
]

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
      bordered
      collapse-mode="width"
      :collapsed-width="64"
      :width="240"
      :collapsed="collapsed"
      show-trigger
      @collapse="collapsed = true"
      @expand="collapsed = false"
    >
      <div class="logo">
        <span v-if="!collapsed">OQQWall</span>
        <span v-else>W</span>
      </div>
      <n-menu
        :collapsed="collapsed"
        :collapsed-width="64"
        :collapsed-icon-size="22"
        :options="menuOptions"
        :value="activeKey"
        @update:value="handleMenuUpdate"
      />
    </n-layout-sider>
    <n-layout>
      <n-layout-header bordered class="header">
        <div class="header-left">
           <h3>{{ menuOptions.find(o => o.key === activeKey)?.label }}</h3>
        </div>
        <div class="header-right">
          <n-dropdown :options="userOptions" @select="handleUserSelect">
            <div class="user-profile">
              <n-avatar round size="small">
                <n-icon><UserIcon /></n-icon>
              </n-avatar>
              <span class="username">{{ auth.me.value?.username }}</span>
            </div>
          </n-dropdown>
        </div>
      </n-layout-header>
      <n-layout-content content-style="padding: 16px; background-color: #f0f2f5; min-height: 100%;">
        <slot></slot>
      </n-layout-content>
    </n-layout>
  </n-layout>
</template>

<style scoped>
.logo {
  height: 64px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 20px;
  font-weight: bold;
  color: #18a058;
  border-bottom: 1px solid #efeff5;
}
.header {
  height: 64px;
  padding: 0 24px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.header-left h3 {
    margin: 0;
    font-weight: 500;
}
.header-right {
  display: flex;
  align-items: center;
}
.user-profile {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 4px;
}
.user-profile:hover {
  background-color: #f3f3f5;
}
.username {
  font-size: 14px;
  font-weight: 500;
}
</style>