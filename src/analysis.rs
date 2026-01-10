use std::ffi::OsString;
use std::sync::atomic::Ordering;
use std::ptr;
use std::os::windows::ffi::OsStringExt;

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
        LAST_IMG_SIZE,
    }, 
    report_error_log, report_info_log
};
use crate::utils::{to_wstring, get_process_info};

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
        if unsafe { OpenClipboard(None).is_ok() } {
            if let Ok(handle) = unsafe { GetClipboardData(CF_HDROP.0 as u32) } {
                // HANDLE 转换为 HDROP
                let h_drop = HDROP(handle.0 as _);
                unsafe { check_dropped_files(h_drop, pid, pname) };
            }
            let _ = unsafe { CloseClipboard() };
            return; 
        }
    }

    // 2. 检查是否是图片内容 (Bitmap)
    let has_bitmap = unsafe { IsClipboardFormatAvailable(CF_BITMAP.0 as u32).is_ok() };
    let has_dib = unsafe { IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() };
    if has_bitmap || has_dib {
        let mut image_data: Vec<u8> = Vec::new();
        let mut data_size: usize = 0; // [新增]

        if unsafe { OpenClipboard(None).is_ok() } {
            if let Ok(handle) = unsafe { GetClipboardData(CF_DIB.0 as u32) } {
                let h_mem = HGLOBAL(handle.0 as _);
                let ptr = unsafe { GlobalLock(h_mem) };
                
                if !ptr.is_null() {
                    // 获取数据大小
                    data_size = unsafe { GlobalSize(h_mem) }; // [新增] 记录大小
                    
                    // === [新增] 防重复过滤核心逻辑 ===
                    let last_size = LAST_IMG_SIZE.load(Ordering::Relaxed);
                    
                    // 如果大小相同，且距离上次更新很近，大概率是关闭程序导致的 Flush
                    // 只有当大小确实大于0时才过滤
                    if data_size > 0 && data_size == last_size {
                        report_info_log!(">> Filtered duplicate image event (Same Size: {} bytes)", data_size);
                        let _unlock = unsafe { GlobalUnlock(h_mem) };
                        let _close = unsafe { CloseClipboard() };
                        return; // 直接返回，不上报
                    }
                    
                    // 更新全局记录
                    LAST_IMG_SIZE.store(data_size, Ordering::Relaxed);
                    // =================================

                    if data_size > 0 {
                        image_data.resize(data_size, 0);
                        unsafe { 
                            ptr::copy_nonoverlapping(ptr as *const u8, image_data.as_mut_ptr(), data_size);
                        }
                    }
                    let _unlock = unsafe { GlobalUnlock(h_mem) };
                }
            }
            let _close = unsafe { CloseClipboard() };
        }

        // 如果没有获取到数据（比如只有 handle 但 lock 失败），也不上报
        if image_data.is_empty() {
            return;
        }

        report_info_log!(">> ALERT: Captured Image. Size: {} bytes. Process: {}", data_size, pname);
        
        let shot_info = ShotInfo {
            pname: pname,
            pid: pid,
            data: image_data.into(), 
        };
        report_shot(shot_info);
        return;
    }
    // 如果不是图片，重置图片大小记录
    LAST_IMG_SIZE.store(0, Ordering::Relaxed);
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