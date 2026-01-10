const { clipboardInitialize } = require('./index.js')

clipboardInitialize(
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
  },
  (err, info) => {
    console.log('日志上报:', info)
  },
)

/**
 * 将 Rust 传来的 DIB 数据 (Uint8Array) 封装为浏览器可识别的 BMP Blob URL
 */
export function createBmpUrlFromDib(dibData) {
  // 确保数据是 Uint8Array
  // 注意：从 Electron IPC 传过来的数据有时会被序列化为普通数组，需强转
  const buffer = dibData instanceof Uint8Array ? dibData : new Uint8Array(dibData)

  // 使用 DataView 读取 DIB 头信息
  const view = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength)

  // --- 1. 解析 DIB 头信息 (BITMAPINFOHEADER) ---
  // DIB 头大小 (通常是 40)
  const biSize = view.getUint32(0, true)
  // 颜色位深 (1, 4, 8, 16, 24, 32)
  const biBitCount = view.getUint16(14, true)
  // 压缩方式 (0=BI_RGB, 3=BI_BITFIELDS)
  const biCompression = view.getUint32(16, true)
  // 实际使用的颜色表中的颜色索引数
  const biClrUsed = view.getUint32(32, true)

  // --- 2. 计算像素数据的起始偏移量 (bfOffBits) ---
  // 标准 BMP 文件头长度总是 14 字节
  // 偏移量 = 14 (文件头) + DIB头大小 + (调色板大小 或 掩码大小)
  let offsetToBits = 14 + biSize

  // 处理调色板 (8位及以下索引颜色)
  if (biBitCount <= 8) {
    // 如果 biClrUsed 为 0，则使用最大颜色数 (1 << biBitCount)
    const paletteCount = biClrUsed === 0 ? 1 << biBitCount : biClrUsed
    offsetToBits += paletteCount * 4 // 每个调色板项 4 字节
  } else if (biCompression === 3) {
    // BI_BITFIELDS: 如果是位域压缩，通常紧跟 3 个 DWORD 掩码 (R, G, B)
    offsetToBits += 12
  }

  // --- 3. 构建 14 字节的 BMP 文件头 (BITMAPFILEHEADER) ---
  const fileHeader = new Uint8Array(14)
  const fhView = new DataView(fileHeader.buffer)

  // [0-1] Magic Number "BM" (0x42, 0x4D)
  fileHeader[0] = 0x42
  fileHeader[1] = 0x4d

  // [2-5] 整个文件的大小 (Header + DIB数据)
  fhView.setUint32(2, 14 + buffer.byteLength, true)

  // [6-9] 保留字，必须为 0
  fhView.setUint32(6, 0, true)

  // [10-13] 像素数据的起始偏移量
  fhView.setUint32(10, offsetToBits, true)

  // --- 4. 拼接 Header + Body 并生成 Blob ---
  // type 必须是 image/bmp
  const blob = new Blob([fileHeader, buffer], { type: 'image/bmp' })

  // 生成 Object URL (例如: blob:http://localhost:3000/...)
  return URL.createObjectURL(blob)
}

setInterval(() => {}, 1000) // 防止进程退出
