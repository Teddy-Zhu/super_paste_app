import { createRouter, createWebHashHistory } from 'vue-router'
import { setupLayouts } from 'virtual:meta-layouts'
import { routes } from 'vue-router/auto-routes' // 引入文件路由表

// 应用布局系统到路由
const layoutRoutes = setupLayouts(routes)

// 创建路由实例
const router = createRouter({
  history: createWebHashHistory(),
  // 直接在创建路由时使用处理好的路由配置
  routes: layoutRoutes
})

// 设置路由守卫
// setupGuards(router)

export default router
