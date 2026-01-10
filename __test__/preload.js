// preload.js
const { contextBridge, ipcRenderer } = require('electron')

contextBridge.exposeInMainWorld('electronAPI', {
  // 暴露一个监听函数，允许前端注册回调
  onUpdateImage: (callback) => ipcRenderer.on('update-image', (_event, value) => callback(value)),
})
