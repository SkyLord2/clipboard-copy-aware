use std::sync::{Mutex};
use std::time::{Duration, Instant};
use windows::{
    core::{ Result, Error },
    Win32::Foundation::{GetLastError, HWND, MAX_PATH, CloseHandle},
    Win32::System::ProcessStatus::GetModuleBaseNameW,
    Win32::UI::WindowsAndMessaging::*,
    Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
};
use crate::global::THROTTLE_MS;

// 辅助：将 Rust 字符串转换为 Windows 宽字符串 (UTF-16)
pub fn to_wstring(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

// 辅助函数：检查是否允许打印（节流逻辑）
#[allow(unused)]
pub fn check_throttle(timer: &Mutex<Option<Instant>>) -> bool {
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

fn last_error() -> Error {
    let code = unsafe { GetLastError() };
    Error::from(Error::from(code))
}

pub unsafe fn get_process_info(hwnd: HWND) -> Result<(u32, String)> {
    let mut pid: u32 = 0;
    // 获取关联的 PID
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };

    if pid == 0 {
        return Err(last_error());
    }

    // 打开进程以查询信息
    // 注意：PROCESS_QUERY_INFORMATION 和 PROCESS_VM_READ 是必须的权限
    let process_handle = unsafe {
        OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)?
    };

    if process_handle.is_invalid() {
        return Err(last_error());
    }

    // 获取进程名
    let mut buffer = [0u16; MAX_PATH as usize];
    // GetModuleBaseNameW 获取的是文件名 (例如 "notepad.exe")
    // 如果需要完整路径，可以使用 GetModuleFileNameExW
    let len = unsafe {
        GetModuleBaseNameW(process_handle, None, &mut buffer)    
    };
    
    // 关闭句柄防止泄露
    let _ = unsafe { CloseHandle(process_handle) };

    if len == 0 {
        return Err(last_error());
    }

    // 将 u16 数组转换为 String
    let name = String::from_utf16_lossy(&buffer[..len as usize]);
    
    Ok((pid, name))
}