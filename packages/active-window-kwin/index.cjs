const path = require('node:path')

let binding
let inFlight

function getSupportStatus() {
  if (process.platform !== 'linux') {
    return { supported: false, reason: 'KWin active window tracking requires Linux.' }
  }
  if (process.env.KDE_SESSION_VERSION !== '6') {
    return { supported: false, reason: 'KWin active window tracking requires KDE Plasma 6.' }
  }
  if (!['x64', 'arm64'].includes(process.arch)) {
    return { supported: false, reason: `Unsupported KWin binding architecture: ${process.arch}` }
  }
  return { supported: true }
}

function getBinding() {
  if (!binding) {
    const file = `active_window_kwin.linux-${process.arch}-gnu.node`
    try {
      binding = require(path.join(__dirname, file))
    } catch (cause) {
      throw new Error(`Unable to load ${file}; build @super-paste/active-window-kwin first.`, {
        cause
      })
    }
  }
  return binding
}

function getActiveWindow() {
  if (inFlight) return inFlight
  const status = getSupportStatus()
  if (!status.supported) return Promise.reject(new Error(status.reason))
  // kdotool generates a temporary script per query. Serialize callers to avoid
  // overlapping queries and upstream's millisecond-based script name collisions.
  inFlight = Promise.resolve()
    .then(() => getBinding().getActiveWindow())
    .then((window) => window ?? null)
    .finally(() => {
      inFlight = undefined
    })
  return inFlight
}

function subscribe(onChange, onError, intervalMs = 500) {
  if (typeof onChange !== 'function' || typeof onError !== 'function') {
    throw new TypeError('subscribe requires onChange and onError callbacks')
  }
  if (!Number.isInteger(intervalMs) || intervalMs < 200 || intervalMs > 60000) {
    throw new RangeError('intervalMs must be an integer between 200 and 60000')
  }
  let active = true
  let timer
  let previous
  const poll = async () => {
    try {
      const window = await getActiveWindow()
      if (!active) return
      const snapshot = JSON.stringify(window)
      if (snapshot !== previous) {
        previous = snapshot
        queueMicrotask(() => {
          if (active) onChange(window)
        })
      }
    } catch (error) {
      previous = undefined
      queueMicrotask(() => {
        if (active) onError(error)
      })
    } finally {
      if (active) {
        timer = setTimeout(poll, intervalMs)
      }
    }
  }
  void poll()
  return () => {
    active = false
    clearTimeout(timer)
  }
}

module.exports = { getSupportStatus, getActiveWindow, subscribe }
