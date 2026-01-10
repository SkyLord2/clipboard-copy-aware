const { clipboardInitialize } = require('./index.js')

clipboardInitialize(
    [
        ".cpp", ".h", ".hpp", ".c", ".cs", ".py", ".java", ".js", ".ts", 
                ".html", ".css", ".json", ".xml", ".sql", ".go", ".rs",
    ], 
    [
        ".jpg", ".jpeg", ".png", ".bmp", ".gif", ".ico", ".tiff", ".webp",
    ], 
    [
        ".xls", ".xlsx", ".csv", ".xlsm",
    ], 
    (err, info) => {
        console.log("文件上报:", info);
    }, 
    (err, info) => {
        console.log("截图上报:", info);
        console.log("图片数据类型：", typeof info.data);
    }, 
    (err, info) => {
        console.log("日志上报:", info);
        
    },
)

setInterval(() => {}, 1000); // 防止进程退出