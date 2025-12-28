#![deny(clippy::all)]

use napi_derive::napi;
use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::System::DataExchange::*,
    Win32::System::Ole::*,
    Win32::System::LibraryLoader::GetModuleHandleW,
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::*,
};

// === 节流控制配置 ===
// 定义节流时间阈值：500毫秒
const THROTTLE_MS: u64 = 500;

// 使用 Mutex 记录上一次打印的时间
// 注意：这里使用 Mutex 是因为我们需要在 immutable 的静态上下文中修改时间
static LAST_UPDATE_LOG: Mutex<Option<Instant>> = Mutex::new(None);


// 辅助函数：检查是否允许打印（节流逻辑）
fn check_throttle(timer: &Mutex<Option<Instant>>) -> bool {
    // 获取锁
    let mut guard = timer.lock().unwrap();
    let now = Instant::now();

    if let Some(last_time) = *guard {
        // 如果距离上次打印的时间小于阈值，则阻止打印
        if now.duration_since(last_time) < Duration::from_millis(THROTTLE_MS) {
            return false;
        }
    }

    // 更新最后打印时间
    *guard = Some(now);
    true
}

fn get_code_extensions() -> &'static HashSet<&'static str> {
    static CODE_EXTENSIONS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    CODE_EXTENSIONS.get_or_init(|| {
        HashSet::from([
            ".cpp", ".h", ".hpp", ".c", ".cs", ".py", ".java", ".js", ".ts", 
            ".html", ".css", ".json", ".xml", ".sql", ".go", ".rs",
        ])
    })
}

fn get_image_extensions() -> &'static HashSet<&'static str> {
    static EXTENSIONS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    EXTENSIONS.get_or_init(|| {
        HashSet::from([
            ".jpg", ".jpeg", ".png", ".bmp", ".gif", ".ico", ".tiff", ".webp",
        ])
    })
}

fn get_excel_extensions() -> &'static HashSet<&'static str> {
    static EXTENSIONS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    EXTENSIONS.get_or_init(|| {
        HashSet::from([
            ".xls", ".xlsx", ".csv", ".xlsm",
        ])
    })
}

// 辅助：将 Rust 字符串转换为 Windows 宽字符串 (UTF-16)
fn to_wstring(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

// 核心逻辑 1: 分析文件列表 (CF_HDROP)
unsafe fn check_dropped_files(h_drop: HDROP) -> Option<String> {
    let file_count = unsafe { DragQueryFileW(h_drop, 0xFFFFFFFF, None) };
    let mut detected_msg = None;

    for i in 0..file_count {
        // 获取路径长度
        let len = unsafe { DragQueryFileW(h_drop, i, None) };
        if len > 0 {
            let mut buffer = vec![0u16; (len + 1) as usize];
            unsafe { DragQueryFileW(h_drop, i, Some(&mut buffer)) };
            
            // 去除结尾的 null 并转为 Path
            if let Some(null_pos) = buffer.iter().position(|&c| c == 0) {
                buffer.truncate(null_pos);
            }
            
            let path_os = OsString::from_wide(&buffer);
            let path = std::path::Path::new(&path_os);

            if let Some(ext_os) = path.extension() {
                if let Some(ext_str) = ext_os.to_str() {
                    // 转小写并添加点号用于匹配
                    let ext = format!(".{}", ext_str.to_lowercase()); 
                    
                    if get_image_extensions().contains(ext.as_str()) {
                        detected_msg = Some(format!(">> ALERT: User copied IMAGE FILE(S): {}", path.display()));
                        break; 
                    } else if get_excel_extensions().contains(ext.as_str()) {
                        detected_msg = Some(format!(">> ALERT: User copied EXCEL FILE(S): {}", path.display()));
                        break;
                    } else if get_code_extensions().contains(ext.as_str()) {
                        detected_msg = Some(format!(">> ALERT: User copied CODE FILE(S): {}", path.display()));
                        // 代码文件的优先级较低，不立即break，除非确定没有图片/表格
                        // (但在本简化逻辑中，只要发现关注文件就break)
                        break;
                    }
                }
            }
        }
    }
    detected_msg
}

// 核心逻辑 2: 分析剪贴板内容
unsafe fn analyze_clipboard() {
    unsafe {
        let owner_hwnd = GetClipboardOwner();
        match owner_hwnd {
            Ok(hwnd) => println!("Clipboard Owner HWND: {:?}", hwnd),
            Err(_) => println!("Clipboard Owner HWND: None"),
        };
    }
    // 1. 检查是否是文件 (CF_HDROP)
    if unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() } {
        if unsafe { OpenClipboard(None).is_ok() } {
            if let Ok(handle) = unsafe { GetClipboardData(CF_HDROP.0 as u32) } {
                // HANDLE 转换为 HDROP
                let h_drop = HDROP(handle.0 as _);
                if let Some(msg) = unsafe { check_dropped_files(h_drop) } {
                    println!(">> {}", msg);
                }
            }
            let _ = unsafe { CloseClipboard() };
            return; 
        }
    }

    // 2. 检查是否是图片内容 (Bitmap)
    let has_bitmap = unsafe { IsClipboardFormatAvailable(CF_BITMAP.0 as u32).is_ok() };
    let has_dib = unsafe { IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() };
    if has_bitmap || has_dib {
        println!(">> ALERT: User copied IMAGE CONTENT (Bitmap/Screenshot).");
        return;
    }

    // 3. 检查是否是表格数据 (HTML / CSV)
    // 注册自定义格式 (只需注册一次，系统会返回相同的ID)
    let format_html = unsafe { RegisterClipboardFormatW(PCWSTR(to_wstring("HTML Format").as_ptr())) };
    let format_csv = unsafe { RegisterClipboardFormatW(PCWSTR(to_wstring("Csv").as_ptr())) };
    let has_html = unsafe { IsClipboardFormatAvailable(format_html).is_ok() };
    let has_csv = unsafe { IsClipboardFormatAvailable(format_csv).is_ok() };
    if has_html || has_csv {
        println!(">> ALERT: User copied TABLE DATA (Cells/HTML).");
        return;
    }
}

// 窗口过程函数
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            if unsafe { AddClipboardFormatListener(hwnd).is_err() } {
                eprintln!("Failed to add clipboard format listener.");
                return LRESULT(-1);
            }
            println!("Monitoring started. Rust is watching your clipboard...");
            println!("Try copying: .jpg files, .xlsx files, Code files, or Screenshots.");
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE => {
            if check_throttle(&LAST_UPDATE_LOG) {
                // println!("-----------------------------------");
                // println!("[Event] Clipboard content changed.");
                unsafe { analyze_clipboard() };    
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = unsafe { RemoveClipboardFormatListener(hwnd) };
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// 辅助函数：处理所有 Windows API 调用，保持其原始错误类型
unsafe fn setup_clipboard_monitor() -> Result<()> {
    let hmodule = GetModuleHandleW(None);
    let class_name = to_wstring("RustClipboardMonitor");

    if let Ok(instance) = hmodule {
        let wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            lpfnWndProc: Some(wnd_proc),
            ..Default::default()
        };

        if RegisterClassW(&wc) == 0 {
            eprintln!("Window Registration Failed!");
            return Err(Error::new(windows::Win32::Foundation::E_FAIL, "Window Registration Failed!"));
        }
        
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(to_wstring("Rust Monitor").as_ptr()),
            WINDOW_STYLE::default(),
            0, 0, 0, 0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()), //将 instance (HMODULE) 转换为 Option<HINSTANCE>
            None,
        );

        if hwnd.is_err() {
            eprintln!("Window Creation Failed!");
            return Err(Error::new(windows::Win32::Foundation::E_FAIL, "Window Creation Failed!"));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    else {
        eprintln!("Failed to get module handle.");
        return Err(Error::new(windows::Win32::Foundation::E_FAIL, "Failed to get module handle."));
    }

    Ok(())
}

#[napi]
pub fn clipboard_initialize() -> napi::Result<()> {
    unsafe {
        let _ = setup_clipboard_monitor();
    };

    Ok(())
}