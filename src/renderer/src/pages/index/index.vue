<script setup lang="ts">
import IconSettings from '~icons/mdi/settings'
import IconKeyboard from '~icons/mdi/keyboard'
import IconInformation from '~icons/mdi/information'
import IconTag from '~icons/mdi/tag'
import IconBug from '~icons/mdi/bug'

// 导入组件
import GeneralSettings from './components/GeneralSettings.vue'
import TagsManagement from './components/TagsManagement.vue'
import ShortcutSettings from './components/ShortcutSettings.vue'
import DebugSettings from './components/DebugSettings.vue'
import AboutSection from './components/AboutSection.vue'

// 当前选中的设置项
const activeSection = ref('general')
const settingsStore = useSettingsStore()

// 设置项列表
const settingSections = [
  {
    id: 'general',
    name: '通用设置',
    icon: IconSettings
  },
  {
    id: 'tags',
    name: '标签管理',
    icon: IconTag
  },
  {
    id: 'shortcuts',
    name: '快捷键',
    icon: IconKeyboard
  },
  {
    id: 'debug',
    name: '调试设置',
    icon: IconBug
  },
  {
    id: 'about',
    name: '关于',
    icon: IconInformation
  }
]

// 切换设置项
const setActiveSection = (sectionId: string) => {
  activeSection.value = sectionId
}
</script>

<template>
  <div class="settings-container">
    <!-- 左侧边栏 -->
    <div class="sidebar">
      <div class="sidebar-header">
        <h2 class="app-title">Super Paste</h2>
        <p class="app-subtitle">设置</p>
      </div>

      <nav class="sidebar-nav">
        <div v-for="section in settingSections" :key="section.id" class="nav-item"
          :class="{ active: activeSection === section.id }" @click="setActiveSection(section.id)">
          <component :is="section.icon" class="nav-icon" />
          <span class="nav-text">{{ section.name }}</span>
        </div>
      </nav>
    </div>

    <!-- 右侧内容区域 -->
    <div class="main-content">
      <!-- 通用设置 -->
      <GeneralSettings v-if="activeSection === 'general'" :settings-store="settingsStore" />

      <!-- 标签管理 -->
      <TagsManagement v-if="activeSection === 'tags'" />

      <!-- 快捷键设置 -->
      <ShortcutSettings v-if="activeSection === 'shortcuts'" :settings-store="settingsStore" />

      <!-- 调试设置 -->
      <DebugSettings v-if="activeSection === 'debug'" :settings-store="settingsStore" />

      <!-- 关于 -->
      <AboutSection v-if="activeSection === 'about'" />
    </div>
  </div>
</template>

<style>
@import './styles/settings.css';
@import './styles/components.css';
@import './styles/tags.css';
</style>
