use std::sync::atomic::{AtomicU32};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use std::collections::HashSet;
use std::fmt;

use napi_derive::napi;
use napi::bindgen_prelude::Uint8Array;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};

// 使用 Mutex 记录上一次打印的时间
// 注意：这里使用 Mutex 是因为我们需要在 immutable 的静态上下文中修改时间
pub static LAST_UPDATE_LOG: Mutex<Option<Instant>> = Mutex::new(None);
// 要监控的文件后缀
pub static CODE_EXTENSIONS: OnceLock<HashSet<String>> = OnceLock::new();
pub static IMAG_EXTENSIONS: OnceLock<HashSet<String>> = OnceLock::new();
pub static EXCE_EXTENSIONS: OnceLock<HashSet<String>> = OnceLock::new();

pub static GLOBAL_REPORT: OnceLock<ThreadsafeFunction<Vec<FileInfo>>> = OnceLock::new();
pub static GLOBAL_REPORT_SHOT: OnceLock<ThreadsafeFunction<ShotInfo>> = OnceLock::new();
pub static GLOBAL_LOG: OnceLock<ThreadsafeFunction<String>> = OnceLock::new();

// 用于记录后台监控线程的 ID
pub static MONITOR_THREAD_ID: AtomicU32 = AtomicU32::new(0);

// === 节流控制配置 ===
// 定义节流时间阈值：500毫秒
pub const THROTTLE_MS: u64 = 500;

// 使用 Mutex 记录上一次的指纹
pub static LAST_IMG_FINGERPRINT: Mutex<Option<ImgFingerprint>> = Mutex::new(None);

#[derive(Clone, Copy,PartialEq, Debug)]
pub struct ImgFingerprint {
    pub width: i32,
    pub height: i32,
    pub center_byte: u8, // 采样点
    pub pid: u32,        // 来源进程
    pub time: Instant, // [新增] 记录产生时间
}

#[napi]
#[derive(Debug)]
pub enum FileType {
    IMAGE,
    CODE,
    EXCEL
}

#[napi(object)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub file_type: FileType,
    pub extension: String,
    pub pname: String,
    pub pid: u32,
}

#[napi(object)]
pub struct ShotInfo {
    pub pname: String,
    pub pid: u32,
    // 图片数据
    pub data: Uint8Array,
}

pub fn report_file(files: Vec<FileInfo>) {
    if let Some(tsfn) = GLOBAL_REPORT.get() {
        tsfn.call(
            Ok(files), ThreadsafeFunctionCallMode::NonBlocking);
    } else {
        println!("Warning: No report wnd listener registered yet!");
    }
}

pub fn report_shot(files: ShotInfo) {
    if let Some(tsfn) = GLOBAL_REPORT_SHOT.get() {
        tsfn.call(
            Ok(files), ThreadsafeFunctionCallMode::NonBlocking);
    } else {
        println!("Warning: No report wnd listener registered yet!");
    }
}

fn report_log(msg: String) {
    if cfg!(debug_assertions) {
        println!("{}", msg);
    } else {
        if let Some(tsfn) = GLOBAL_LOG.get() {
            tsfn.call(Ok(msg), ThreadsafeFunctionCallMode::NonBlocking);
        } else {
            println!("Warning: No report log listener registered yet!");
        }
    }
}

#[doc(hidden)]
pub(crate) fn report_error(msg: fmt::Arguments) {
  let log_msg = format!("[clipboard_error]: {}", msg);
  report_log(log_msg);
}

#[doc(hidden)]
pub(crate) fn report_info(msg: fmt::Arguments) {
  let log_msg = format!("[clipboard_info]: {}", msg);
  report_log(log_msg);
}

#[macro_export]
macro_rules! report_error_log {
    // format_args! 是编译器内置宏，它不分配内存，只打包参数
    ($($arg:tt)*) => {
        $crate::global::report_error(format_args!($($arg)*))
    }
}

#[macro_export]
macro_rules! report_info_log {
    // format_args! 是编译器内置宏，它不分配内存，只打包参数
    ($($arg:tt)*) => {
        $crate::global::report_info(format_args!($($arg)*))
    }
}

pub fn get_code_extensions() -> &'static HashSet<String> {
    CODE_EXTENSIONS.get_or_init(|| {
        HashSet::from(
            [
                ".cpp", ".h", ".hpp", ".c", ".cs", ".py", ".java", ".js", ".ts", 
                ".html", ".css", ".json", ".xml", ".sql", ".go", ".rs",
            ].map(String::from)
        )
    })
}

pub fn get_image_extensions() -> &'static HashSet<String> {
    IMAG_EXTENSIONS.get_or_init(|| {
        HashSet::from(
            [
                ".jpg", ".jpeg", ".png", ".bmp", ".gif", ".ico", ".tiff", ".webp",
            ].map(String::from)
        )
    })
}

pub fn get_excel_extensions() -> &'static HashSet<String> {
    EXCE_EXTENSIONS.get_or_init(|| {
        HashSet::from(
            [
                ".xls", ".xlsx", ".csv", ".xlsm",
            ].map(String::from)
        )
    })
}
