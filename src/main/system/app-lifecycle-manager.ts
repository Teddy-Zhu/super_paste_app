import { app } from 'electron'
import { mainWindowManager } from '../ui/windows/window-managers'
import logger from '../utils/logger'

class AppLifecycleManager {
    private static instance: AppLifecycleManager
    private isQuitting: boolean = false

    private constructor() { }

    public static getInstance(): AppLifecycleManager {
        if (!AppLifecycleManager.instance) {
            AppLifecycleManager.instance = new AppLifecycleManager()
        }
        return AppLifecycleManager.instance
    }

    /**
     * Hide the dock icon (macOS only)
     */
    public hideDock(): void {
        if (process.platform === 'darwin') {
            app.dock?.hide()
        }
    }

    /**
     * Show the dock icon (macOS only)
     */
    public showDock(): void {
        if (process.platform === 'darwin') {
            app.dock?.show()
        }
    }

    /**
     * Handle window all closed event
     */
    public handleWindowAllClosed(): void {
        logger.debug('[AppLifecycle] All windows closed')
        // On macOS, applications typically stay open until the user quits explicitly
        // Since we have a tray, we generally stay open on other platforms too
    }

    /**
     * Handle before-quit event
     */
    public handleBeforeQuit(): void {
        this.isQuitting = true
        logger.info('[AppLifecycle] Before quit event received')
    }

    /**
     * Handle application activation (e.g. clicking dock icon)
     */
    public handleActivate(isInitialized: boolean): void {
        if (!isInitialized) return
        mainWindowManager.showWindow()
    }

    public getIsQuitting(): boolean {
        return this.isQuitting
    }
}

export const appLifecycleManager = AppLifecycleManager.getInstance()
