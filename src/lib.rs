#![deny(clippy::all)]
pub mod global;
mod utils;
mod hooks;
mod analysis;

use napi_derive::napi;
use napi::{ Env, Status };
use napi::threadsafe_function::{ThreadsafeFunction};

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::ffi::c_void;
use std::thread;

use windows::{
    Win32::Foundation::{WPARAM, LPARAM},
    Win32::System::Threading::GetCurrentThreadId,
    Win32::UI::WindowsAndMessaging::{ 
        PostThreadMessageW, WM_QUIT
    },
};

use crate::global::{
   FileInfo, ShotInfo, CODE_EXTENSIONS, EXCE_EXTENSIONS, GLOBAL_LOG, GLOBAL_REPORT, GLOBAL_REPORT_SHOT, IMAG_EXTENSIONS, MONITOR_THREAD_ID,
};
use crate::hooks::setup_clipboard_monitor;

unsafe extern "C" fn cleanup_monitor_thread(_arg: *mut c_void) {
    let thread_id = MONITOR_THREAD_ID.load(Ordering::SeqCst);
    if thread_id != 0 {
        // 向后台线程发送 WM_QUIT，打破它的死循环
        let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        println!("Cleanup hook triggered: Sent WM_QUIT to monitor thread.");
    }
}

#[napi]
pub fn clipboard_initialize(
    code_exts: Vec<String>, 
    imag_exts: Vec<String>, 
    exce_exts: Vec<String>, 
    mut report_file: ThreadsafeFunction<Vec<FileInfo>>,
    mut report_shot: ThreadsafeFunction<ShotInfo>,
    mut log: ThreadsafeFunction<String>,
    env: Env
) -> napi::Result<()> {
    #[allow(deprecated)]
    report_file.unref(&env)?;
    #[allow(deprecated)]
    report_shot.unref(&env)?;
    #[allow(deprecated)]
    log.unref(&env)?;

    CODE_EXTENSIONS.get_or_init(|| {HashSet::<String>::from_iter(code_exts)});
    IMAG_EXTENSIONS.get_or_init(|| {HashSet::<String>::from_iter(imag_exts)});
    EXCE_EXTENSIONS.get_or_init(|| {HashSet::<String>::from_iter(exce_exts)});

    GLOBAL_REPORT.set(report_file).map_err(|_| napi::Error::new(Status::GenericFailure, "Global report file listener already registered"))?;
    GLOBAL_REPORT_SHOT.set(report_shot).map_err(|_| napi::Error::new(Status::GenericFailure, "Global report shot listener already registered"))?;
    GLOBAL_LOG.set(log).map_err(|_| napi::Error::new(Status::GenericFailure, "Global log listener already registered"))?;

    if cfg!(debug_assertions) {
        report_info_log!("[Debug] 当前正处于开发模式运行，开启详细日志...");
    } else {
        report_info_log!("[Release] 生产模式运行");
    }

    env.add_env_cleanup_hook(
        std::ptr::null_mut(), 
        |arg| unsafe { cleanup_monitor_thread(arg) }
    )?;

    thread::spawn(move || {
        unsafe {
            let thread_id = GetCurrentThreadId();
            MONITOR_THREAD_ID.store(thread_id, Ordering::SeqCst);
            let _ = setup_clipboard_monitor().map_err(|e| {
                napi::Error::from_reason(format!(
                    "Clipboard Monitor Failed: {} (Code: 0x{:X})", 
                    e.message(), 
                    e.code().0
                ))
            });
        }
    });

    Ok(())
}