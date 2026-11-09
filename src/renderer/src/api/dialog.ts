import { confirm as nativeConfirm, message as nativeMessage } from '@tauri-apps/plugin-dialog'
import { DIALOG_TEXT } from '@renderer/constants/dialog'

/** Returns whether the native message dialog completed successfully. */
export async function showMessage(message: string): Promise<boolean> {
  try {
    await nativeMessage(message, { title: DIALOG_TEXT.title, okLabel: DIALOG_TEXT.confirm })
    return true
  } catch (error) {
    console.error(DIALOG_TEXT.messageFailed, error)
    return false
  }
}

/** Cancellation or dialog failure cannot authorize a destructive operation. */
export async function confirmAction(message: string): Promise<boolean> {
  try {
    return await nativeConfirm(message, {
      title: DIALOG_TEXT.title,
      okLabel: DIALOG_TEXT.confirm,
      cancelLabel: DIALOG_TEXT.cancel
    })
  } catch (error) {
    console.error(DIALOG_TEXT.confirmFailed, error)
    return false
  }
}
