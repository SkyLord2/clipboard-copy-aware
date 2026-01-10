use windows::{
    core::{ PCWSTR, Result, Error },
    Win32::Foundation::*,
    Win32::System::DataExchange::*,
    Win32::System::LibraryLoader::GetModuleHandleW,
    Win32::UI::WindowsAndMessaging::*,
};
use crate::{report_error_log, report_info_log, utils::{check_throttle, to_wstring}};
use crate::global::LAST_UPDATE_LOG;
use crate::analysis::analyze_clipboard;
// 窗口过程函数
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            if unsafe { AddClipboardFormatListener(hwnd).is_err() } {
                report_error_log!("Failed to add clipboard format listener.");
                return LRESULT(-1);
            }
            report_info_log!("Monitoring started. Rust is watching your clipboard...");
            report_info_log!("Try copying: .jpg files, .xlsx files, Code files, or Screenshots.");
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE => {
            if check_throttle(&LAST_UPDATE_LOG) {
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
pub unsafe fn setup_clipboard_monitor() -> Result<()> {
    let instance = GetModuleHandleW(None)?;
    let class_name = to_wstring("RustClipboardMonitor");

    
    let wc = WNDCLASSW {
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        hInstance: instance.into(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        lpfnWndProc: Some(wnd_proc),
        ..Default::default()
    };

    if RegisterClassW(&wc) == 0 {
        report_error_log!("Window Registration Failed!");
        return Err(Error::from(unsafe { GetLastError() }));
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
        report_error_log!("Window Creation Failed!");
        return Err(Error::from(unsafe { GetLastError() }));
    }

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    Ok(())
}