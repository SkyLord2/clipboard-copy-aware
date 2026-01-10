use std::ffi::OsString;
use std::ptr;
use std::os::windows::ffi::OsStringExt;
use std::thread; // [新增]
use std::time::{Duration, Instant}; // [新增]

use chrono::Local;

use windows::{
    core::{ PCWSTR },
    Win32::Foundation::{HGLOBAL},
    Win32::System::DataExchange::*,
    Win32::System::Ole::*,
    Win32::UI::Shell::*,
    Win32::System::Memory::{GlobalLock, GlobalUnlock, GlobalSize},
};

use crate::{
    global::{
        FileInfo, FileType, ShotInfo, get_code_extensions, get_excel_extensions, get_image_extensions, report_file, report_shot,
        LAST_IMG_FINGERPRINT, ImgFingerprint,
    }, 
    report_error_log, report_info_log
};
use crate::utils::{to_wstring, get_process_info};

// 辅助函数：带重试机制的 OpenClipboard
// 尝试打开 10 次，每次间隔 10ms，总共等待 100ms
// 解决剪贴板竞争问题
unsafe fn try_open_clipboard() -> bool {
    for _ in 0..10 {
        if OpenClipboard(None).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

// 核心逻辑 1: 分析文件列表 (CF_HDROP)
unsafe fn check_dropped_files(h_drop: HDROP, pid: u32, pname: String) {
    let file_count = unsafe { DragQueryFileW(h_drop, 0xFFFFFFFF, None) };
    let mut detected_msg = String::from("no file detected");

    let mut files: Vec<FileInfo> = vec![];
    report_info_log!("detected file count: {}", file_count);
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

            let file_name = path.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();

            let path_str = path.to_string_lossy().into_owned();

            if let Some(ext_os) = path.extension() {
                if let Some(ext_str) = ext_os.to_str() {
                    // 转小写并添加点号用于匹配
                    let ext = format!(".{}", ext_str.to_lowercase()); 
                    
                    let detected_type = if get_image_extensions().contains(&ext) {
                        detected_msg = format!(">> ALERT: User copied IMAGE FILE(S): {}", path.display());
                        Some(FileType::IMAGE)
                    } else if get_excel_extensions().contains(&ext) {
                        detected_msg = format!(">> ALERT: User copied EXCEL FILE(S): {}", path.display());
                        Some(FileType::EXCEL)
                    } else if get_code_extensions().contains(&ext) {
                        detected_msg = format!(">> ALERT: User copied CODE FILE(S): {}", path.display());
                        // 代码文件的优先级较低，不立即break，除非确定没有图片/表格
                        // (但在本简化逻辑中，只要发现关注文件就break)
                        Some(FileType::CODE)
                    } else {
                        None
                    };
                    // [新增] 如果是关注的文件类型，推入数组
                    if let Some(ft) = detected_type {
                        files.push(FileInfo {
                            name: file_name,
                            path: path_str,
                            file_type: ft,
                            extension: ext,
                            pname: pname.clone(), // [新增]
                            pid: pid,             // [新增]
                        });
                    }
                    report_info_log!("{}", detected_msg);
                }
            }
        }
    }
    if !files.is_empty() {
        report_file(files);
    }
}

// 核心逻辑 2: 分析剪贴板内容
pub unsafe fn analyze_clipboard() {
    let mut pid: u32 = 0;
    let mut pname: String = "Unknown".to_string();
    let now = Instant::now();
    unsafe {
        let owner_hwnd = GetClipboardOwner();
        match owner_hwnd {
            Ok(hwnd) => {
                report_info_log!("Clipboard Owner HWND: {:?}", hwnd);
                if let Ok((p, n)) = get_process_info(hwnd) {
                    pid = p;
                    pname = n;
                    report_info_log!("Source Process: {} (PID: {})", pname, pid);
                }
            },
            Err(_) => report_error_log!("Clipboard Owner HWND: None"),
        };
    }
    // 1. 检查是否是文件 (CF_HDROP)
    if unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() } {
        if unsafe { try_open_clipboard() } {
            if let Ok(handle) = unsafe { GetClipboardData(CF_HDROP.0 as u32) } {
                let h_drop = HDROP(handle.0 as _);
                unsafe { check_dropped_files(h_drop, pid, pname) };
            }
            let _ = unsafe { CloseClipboard() };
            return; 
        } else {
            // [新增] 错误日志
            report_error_log!("Failed to OpenClipboard for File check (Occupied by other app).");
        }
    }

    // 2. 检查是否是图片内容 (Bitmap)
    let has_bitmap = unsafe { IsClipboardFormatAvailable(CF_BITMAP.0 as u32).is_ok() };
    let has_dib = unsafe { IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() };
    if has_bitmap || has_dib {
        if pid == 0 {
            report_info_log!("pid is 0.");
            if let Ok(mut guard) = LAST_IMG_FINGERPRINT.lock() {
                *guard = None;
            }
            return; 
        }
        if unsafe { IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() } {
            let mut image_data: Vec<u8> = Vec::new();

            if unsafe { try_open_clipboard() } {
                if let Ok(handle) = unsafe { GetClipboardData(CF_DIB.0 as u32) } {
                    let h_mem = HGLOBAL(handle.0 as _);
                    let ptr = unsafe { GlobalLock(h_mem) } as *const u8;
                    
                    if !ptr.is_null() {
                        let data_size = unsafe { GlobalSize(h_mem) };
                        
                        // BITMAPINFOHEADER 至少 40 字节
                        if data_size > 40 {
                            // === [核心去重逻辑] ===
                            
                            // Windows BITMAPINFOHEADER 结构:
                            // Offset 0: biSize (u32)
                            // Offset 4: biWidth (i32)  <-- 读取这个
                            // Offset 8: biHeight (i32) <-- 读取这个
                            
                            let width = unsafe { ptr::read_unaligned(ptr.add(4) as *const i32) };
                            let height = unsafe { ptr::read_unaligned(ptr.add(8) as *const i32) };
                            
                            // 采样：读取数据中间的一个字节。
                            // 即使两张图都是 1920x1080，如果内容不同，中间这个字节大概率不同。
                            let center_byte = unsafe { *ptr.add(data_size / 2) };

                            let current_fp = ImgFingerprint {
                                width,
                                height,
                                center_byte,
                                pid,
                                time: now
                            };

                            let is_duplicate = {
                                let mut guard = LAST_IMG_FINGERPRINT.lock().unwrap();
                                if let Some(last) = *guard {
                                    // 判定条件 1: 完全一致（宽、高、采样、PID都一样） -> 重复
                                    let is_exact_match = last.pid == pid && last.width == width && last.height == height && last.center_byte == center_byte;
                                    // 判定条件 2 (新增): PID 相同，且距离上次截图时间 < 600ms -> 视为截图工具的抖动/微调 -> 重复
                                    // 人无法在 600ms 内完成两次有效的不同截图操作
                                    let is_rapid_update = last.pid == pid && now.duration_since(last.time) < Duration::from_millis(600);
                                    // 如果 宽、高、采样点、PID 全部一样 -> 判定为重复
                                    if is_exact_match || is_rapid_update {
                                        // 如果是快速更新，我们甚至可以更新一下最后的时间，延长防抖窗口
                                        // *guard = Some(current_fp); // 可选：更新为最新的（如果你想保留最后一次）
                                        // 但这里我们选择保留第一次的指纹，直接丢弃后续的微调
                                        true 
                                    } else {
                                        *guard = Some(current_fp);
                                        false
                                    }
                                } else {
                                    *guard = Some(current_fp);
                                    false
                                }
                            };

                            if is_duplicate {
                                report_info_log!(">> {} Filtered duplicate Image ({}x{}, PID: {})", Local::now().format("%Y-%m-%d %H:%M:%S"), width, height, pid);
                                let _ = unsafe { GlobalUnlock(h_mem) };
                                let _arg = unsafe { CloseClipboard() };
                                return; // [直接退出，不上报]
                            } else {
                                report_info_log!(">> {} New Image detected ({}x{}, PID: {})", Local::now().format("%Y-%m-%d %H:%M:%S"), width, height, pid);
                            }
                            // ======================

                            // 数据不重复，执行拷贝
                            image_data.resize(data_size, 0);
                            unsafe { 
                                ptr::copy_nonoverlapping(ptr, image_data.as_mut_ptr(), data_size);
                            }
                        }
                        let _ = unsafe { GlobalUnlock(h_mem) };
                    }
                }
                let _arg = unsafe { CloseClipboard() };
            } else {
                // [新增] 错误日志
                report_error_log!("Failed to OpenClipboard for Image check (Occupied by other app).");
            }

            if !image_data.is_empty() {
                report_info_log!(">> ALERT: Captured Image. Process: {}", pname);
                let shot_info = ShotInfo {
                    pname: pname,
                    pid: pid,
                    data: image_data.into(), 
                };
                report_shot(shot_info);
            }
            return;
        }
    } else {
        report_info_log!("No bitmap data in clipboard.");
    }
    // 3. 其他类型 (清空指纹)
    if let Ok(mut guard) = LAST_IMG_FINGERPRINT.lock() {
        *guard = None;
    }
    // 3. 检查是否是表格数据 (HTML / CSV)
    // 注册自定义格式 (只需注册一次，系统会返回相同的ID)
    let format_html = unsafe { RegisterClipboardFormatW(PCWSTR(to_wstring("HTML Format").as_ptr())) };
    let format_csv = unsafe { RegisterClipboardFormatW(PCWSTR(to_wstring("Csv").as_ptr())) };
    let has_html = unsafe { IsClipboardFormatAvailable(format_html).is_ok() };
    let has_csv = unsafe { IsClipboardFormatAvailable(format_csv).is_ok() };
    if has_html || has_csv {
        report_info_log!(">> ALERT: User copied TABLE DATA (Cells/HTML).");
        return;
    }
}