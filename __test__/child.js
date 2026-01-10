// child.js
console.log('子进程启动，PID:', process.pid)
let nativeAddon
try {
  console.log('正在加载原生扩展...')

  nativeAddon = require('clipboard-copy-aware')

  console.log('原生扩展加载成功: ', nativeAddon)

  if (typeof nativeAddon.clipboardInitialize === 'function') {
    console.log('找到 clipboardInitialize 方法')
  }

  nativeAddon.clipboardInitialize(
    [
      '.cpp',
      '.h',
      '.hpp',
      '.c',
      '.cs',
      '.py',
      '.java',
      '.js',
      '.ts',
      '.html',
      '.css',
      '.json',
      '.xml',
      '.sql',
      '.go',
      '.rs',
    ],
    ['.jpg', '.jpeg', '.png', '.bmp', '.gif', '.ico', '.tiff', '.webp'],
    ['.xls', '.xlsx', '.csv', '.xlsm'],
    (err, info) => {
      console.log('文件上报:', info)
    },
    (err, info) => {
      console.log('截图上报:', info)
      console.log('图片数据类型：', typeof info.data)
      if (info && info.data) {
        // 手动将 Rust 传过来的 Uint8Array 转为 Node.js Buffer
        // 这样 process.send 就会把它序列化为 { type: 'Buffer', data: [...] }
        // 而不是那几十万行的键值对对象
        info.data = Buffer.from(info.data)
      }
      process.send({
        taskId: 123456,
        info,
        error: err,
      })
    },
    (err, info) => {
      console.log('日志上报:', info)
    },
  )
} catch (error) {
  console.error('加载原生扩展失败:', error.message)
}

// 监听父进程的消息
process.on('message', async (message) => {
  console.log('子进程收到消息:', message)
  try {
    // 发送结果回父进程
    process.send({
      taskId: message.id,
      result: result,
      timestamp: new Date().toISOString(),
    })
  } catch (error) {
    console.error('处理任务时出错:', error)
    process.send({
      taskId: message.id,
      error: error.message,
    })
  }
})

// 发送就绪信号给父进程
process.send('ready')

console.log('子进程等待消息中...')
