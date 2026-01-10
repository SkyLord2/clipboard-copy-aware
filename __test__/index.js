const { app, BrowserWindow } = require('electron')
const { fork } = require('child_process')
const path = require('path')

const createWindow = () => {
  // 创建浏览器窗口
  const win = new BrowserWindow({
    width: 800,
    height: 600,
    webPreferences: {
      // 预加载脚本（可选，用于安全地暴露 Node API 给页面）
      preload: path.join(__dirname, 'preload.js') 
    }
  })

  // 加载 index.html
  win.loadFile('index.html')
  const child = fork(path.join(__dirname, 'child.js'));
  child.on('message', (message) => {
    console.log('主进程收到消息:', message);
    if (message.info && message.info.data) {
        if (message.info.data.type === 'Buffer' && Array.isArray(message.info.data.data)) {
          // 2. 还原为真正的 Buffer 对象
          message.info.data = Buffer.from(message.info.data.data);
        }
        
        // 3. 发送给前端 (Electron 会高效处理 Buffer)
        win.webContents.send('update-image', message.info);
    }
    if (message === 'exit') {
      console.log('正在关闭子进程...');
      child.kill();
    }
  });
  child.on('error', (error) => {
    console.error('子进程错误:', error);
  });
  child.on('exit', (code, signal) => {
    console.log(`子进程已退出，代码: ${code}, 信号: ${signal}`);
  });
}

// 当 Electron 完成初始化时调用
app.whenReady().then(() => {
  createWindow()

  // macOS 特性：如果没有窗口打开，激活应用时重新创建窗口
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

// 关闭所有窗口时退出应用（Windows/Linux）
app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit()
})