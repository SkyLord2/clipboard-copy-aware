//
use std::ffi::OsString;
use std::ptr;
use std::os::windows::ffi::OsStringExt;
use std::thread;
use std::time::{Duration, Instant};

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
    let file_count = DragQueryFileW(h_drop, 0xFFFFFFFF, None);
    let mut files: Vec<FileInfo> = vec![];
    
    report_info_log!("detected file count: {}", file_count);

    for i in 0..file_count {
        // 1. 获取长度，如果是0直接跳过
        let len = DragQueryFileW(h_drop, i, None);
        if len == 0 { continue; }

        // 2. 获取文件名
        let mut buffer = vec![0u16; (len + 1) as usize];
        DragQueryFileW(h_drop, i, Some(&mut buffer));
        
        if let Some(null_pos) = buffer.iter().position(|&c| c == 0) {
            buffer.truncate(null_pos);
        }
        
        let path_os = OsString::from_wide(&buffer);
        let path = std::path::Path::new(&path_os);

        // 3. 提取文件名和扩展名 (使用 Guard Clauses 减少嵌套)
        let file_name = path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path_str = path.to_string_lossy().into_owned();

        let Some(ext_os) = path.extension() else { continue };
        let Some(ext_str) = ext_os.to_str() else { continue };
        
        let ext = format!(".{}", ext_str.to_lowercase()); 
        
        // 4. 判断类型
        let mut detected_msg = String::new();
        let detected_type = if get_image_extensions().contains(&ext) {
            detected_msg = format!(">> ALERT: User copied IMAGE FILE(S): {}", path.display());
            Some(FileType::IMAGE)
        } else if get_excel_extensions().contains(&ext) {
            detected_msg = format!(">> ALERT: User copied EXCEL FILE(S): {}", path.display());
            Some(FileType::EXCEL)
        } else if get_code_extensions().contains(&ext) {
            detected_msg = format!(">> ALERT: User copied CODE FILE(S): {}", path.display());
            Some(FileType::CODE)
        } else {
            None
        };

        if let Some(ft) = detected_type {
            files.push(FileInfo {
                name: file_name,
                path: path_str,
                file_type: ft,
                extension: ext,
                pname: pname.clone(),
                pid: pid,
            });
            report_info_log!("{}", detected_msg);
        }
    }

    if !files.is_empty() {
        report_file(files);
    }
}

// [新增] 抽离图片处理逻辑，大幅减少 analyze_clipboard 的嵌套深度
unsafe fn process_image_content(pid: u32, pname: String) {
    if pid == 0 {
        report_info_log!("pid is 0.");
        // 如果是系统接管（通常意味着应用关闭），清除指纹防止逻辑干扰
        if let Ok(mut guard) = LAST_IMG_FINGERPRINT.lock() {
            *guard = None;
        }
        return;
    }

    // 1. 检查格式：必须有 DIB 数据
    if !IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() {
        report_info_log!("No DIB format available in clipboard.");
        return;
    }

    // 2. 尝试打开剪贴板
    if !try_open_clipboard() {
        report_error_log!("Failed to OpenClipboard for Image check (Occupied by other app).");
        return;
    }

    // 使用闭包或单独的代码块来处理数据读取，以便统一进行 CloseClipboard
    // 这里的返回值是 Option<Vec<u8>>，如果成功读取则返回数据，否则 None
    let image_data_opt = (|| {
        // Guard Clause: 获取句柄
        let Ok(handle) = GetClipboardData(CF_DIB.0 as u32) else { return None; };
        
        let h_mem = HGLOBAL(handle.0 as _);
        let ptr = GlobalLock(h_mem) as *const u8;
        if ptr.is_null() { return None; }

        // 注意：GlobalLock 后，必须在返回前 GlobalUnlock
        // 为了安全，接下来的所有 return 路径都必须 Unlock
        
        let data_size = GlobalSize(h_mem);
        if data_size <= 40 {
            let _ = GlobalUnlock(h_mem);
            return None;
        }

        // --- 核心去重逻辑 ---
        let width = ptr::read_unaligned(ptr.add(4) as *const i32);
        let height = ptr::read_unaligned(ptr.add(8) as *const i32);
        let center_byte = *ptr.add(data_size / 2);
        let now = Instant::now();

        let current_fp = ImgFingerprint { width, height, center_byte, pid, time: now };

        let is_duplicate = {
            let mut guard = LAST_IMG_FINGERPRINT.lock().unwrap();
            if let Some(last) = *guard {
                let is_exact_match = last.pid == pid && last.width == width && last.height == height && last.center_byte == center_byte;
                let is_rapid_update = last.pid == pid && now.duration_since(last.time) < Duration::from_millis(600);
                
                if is_exact_match || is_rapid_update {
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
            let _ = GlobalUnlock(h_mem);
            return None;
        }

        report_info_log!(">> {} New Image detected ({}x{}, PID: {})", Local::now().format("%Y-%m-%d %H:%M:%S"), width, height, pid);

        // --- 拷贝数据 ---
        let mut data = vec![0u8; data_size];
        ptr::copy_nonoverlapping(ptr, data.as_mut_ptr(), data_size);
        
        let _ = GlobalUnlock(h_mem);
        Some(data)
    })();

    // 统一关闭剪贴板
    let _ = CloseClipboard();

    // 3. 上报数据
    if let Some(data) = image_data_opt {
        report_info_log!(">> ALERT: Captured Image. Process: {}", pname);
        report_shot(ShotInfo {
            pname,
            pid,
            data: data.into(), 
        });
    }
}

// 核心逻辑 2: 分析剪贴板内容 (主入口)
pub unsafe fn analyze_clipboard() {
    let mut pid: u32 = 0;
    let mut pname: String = "Unknown".to_string();

    unsafe {
        if let Ok(hwnd) = GetClipboardOwner() {
            // report_info_log!("Clipboard Owner HWND: {:?}", hwnd);
            if let Ok((p, n)) = get_process_info(hwnd) {
                pid = p;
                pname = n;
                report_info_log!("Source Process: {} (PID: {})", pname, pid);
            }
        }
    }

    // 1. 检查是否是文件 (CF_HDROP)
    if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() {
        if try_open_clipboard() {
            if let Ok(handle) = GetClipboardData(CF_HDROP.0 as u32) {
                check_dropped_files(HDROP(handle.0 as _), pid, pname);
            }
            let _ = CloseClipboard();
        } else {
            report_error_log!("Failed to OpenClipboard for File check (Occupied by other app).");
        }
        return; // 文件处理完毕，直接返回
    }

    // 2. 检查是否是图片内容 (Bitmap / DIB)
    let has_bitmap = IsClipboardFormatAvailable(CF_BITMAP.0 as u32).is_ok();
    let has_dib = IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok();
    
    if has_bitmap || has_dib {
        process_image_content(pid, pname);
        return; // 图片处理完毕，直接返回
    } else {
        // 如果不是图片，清空指纹，防止误判
        if let Ok(mut guard) = LAST_IMG_FINGERPRINT.lock() {
            *guard = None;
        }
    }

    // 3. 检查是否是表格数据 (HTML / CSV)
    let format_html = RegisterClipboardFormatW(PCWSTR(to_wstring("HTML Format").as_ptr()));
    let format_csv = RegisterClipboardFormatW(PCWSTR(to_wstring("Csv").as_ptr()));
    
    let has_html = IsClipboardFormatAvailable(format_html).is_ok();
    let has_csv = IsClipboardFormatAvailable(format_csv).is_ok();
    
    if has_html || has_csv {
        report_info_log!(">> ALERT: User copied TABLE DATA (Cells/HTML).");
        return;
    }
    
    report_info_log!("No bitmap/file data in clipboard.");
}