// Preserve the renderer API while replacing Electron IPC with Tauri commands.
if (window.__TAURI__) {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const call = (command, args) => invoke(command, args);
  const event = (name, callback) => listen(name, (message) => callback(message.payload));
  function withPace(state) {
    if (!state?.quota || typeof buildPaceAdvice !== "function") return state;
    state.quota.paceAdvice = buildPaceAdvice(state.quota, state.quota.fetchedAt, state.history || []);
    return state;
  }
  window.codexQuota = {
    getQuotaState: () => call("get_quota_state").then(withPace),
    refreshQuota: () => call("refresh_quota").then(withPace),
    minimize: () => call("hide_window"),
    close: () => call("quit_app"),
    getAlwaysOnTop: () => call("get_always_on_top"),
    setAlwaysOnTop: (value) => call("set_always_on_top", { value }),
    getCompactMode: () => call("get_compact_mode"),
    setCompactMode: (value) => call("set_compact_mode", { value }),
    getCompactScale: () => call("get_compact_scale"),
    setCompactScale: (value) => call("set_compact_scale", { value }),
    moveCompactWindow: (deltaX, deltaY) => call("move_compact_window", { deltaX, deltaY }),
    snapCompactWindow: () => call("snap_compact_window"),
    setCompactExpanded: (value) => call("set_compact_expanded", { value }),
    getCompactDisplayMode: () => call("get_compact_display_mode"),
    setCompactMousePassthrough: (value) => call("set_compact_mouse_passthrough", { value }),
    getCursorState: () => call("get_cursor_state"),
    getSignalSettings: () => call("get_signal_settings"),
    setSignalSettings: (settings) => call("set_signal_settings", { settings }),
    resetSignalSettings: () => call("reset_signal_settings"),
    onQuotaChanged: (callback) => event("quota:changed", (value) => callback(withPace(value))),
    onAlwaysOnTopChanged: (callback) => event("window:alwaysOnTopChanged", callback),
    onCompactChanged: (callback) => event("window:compactChanged", callback),
    onCompactScaleChanged: (callback) => event("window:compactScaleChanged", callback),
    onCompactDisplayModeChanged: (callback) => event("window:compactDisplayModeChanged", callback),
    onSignalSettingsChanged: (callback) => event("signal:settingsChanged", callback)
  };
}
