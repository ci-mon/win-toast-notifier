use std::ptr::null_mut;
use std::{env, fmt, str, thread};
use std::ffi::{CString, OsStr};
use std::fmt::{Display, format, Formatter, Pointer};
use std::os::windows::ffi::OsStrExt;
use std::process::{Command, Stdio};
use named_pipe::PipeClient;
use std::fs::File;

use windows::Win32::System::Console::{SetStdHandle, STD_OUTPUT_HANDLE};
use windows::Win32::Storage::FileSystem;
use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS};
use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
use std::ptr;
use std::os::windows::io::AsRawHandle;
use std::io::{Read, stdout, Write};
use std::sync::{Arc, LockResult, Mutex};
use std::thread::sleep;
use std::time::Duration;
use lazy_static::lazy_static;
use serde_json::to_string;
use tokio::task::JoinHandle;
use windows::core::{PCWSTR, w};
macro_rules! println_pipe {
    ($($arg:tt)*) => {{
        use std::io::{stdout, Write};
        use crate::elevator_values::elevator_values;
        let line = format!($($arg)*);
        match elevator_values::OUT_PIPE.lock() {
            Ok(mut guard) => {
                if let Some(pipe) = guard.as_mut() {
                    _ = pipe.write_all(line.to_string().as_bytes());
                    _ = pipe.write_all("\n".to_string().as_bytes());
                }
            }
            Err(_) => {}
        }
        println!($($arg)*);
    }};
}
pub(crate) use println_pipe;


#[test]
fn test_redirect() {
    enable_pipe_output("win-toast-notifier".to_string());
    println_pipe!("test");
}

#[tokio::test]
async fn test_dump_pipe() {
    dump_pipe("win-toast-notifier".to_string()).join().unwrap();
}

pub fn enable_pipe_output(pipe_name: String) {
    let pipe_name = get_pipe_name(pipe_name);
    if let Ok(pipe) = PipeClient::connect(pipe_name) {
        match crate::elevator_values::elevator_values::OUT_PIPE.lock() {
            Ok(mut guard) => {
                guard.replace(pipe);
            }
            Err(_) => {}
        }
    }
}

fn get_pipe_name(pipe_name: String) -> String {
    format!(r"\\.\pipe\{}", pipe_name)
}

pub fn dump_pipe(pipe_name: String) -> thread::JoinHandle<()> {
    thread::spawn(|| {
        let pipe_name = get_pipe_name(pipe_name);
        let mut pipe = named_pipe::PipeOptions::new(pipe_name)
            .single().unwrap()
            .wait().unwrap();
        let mut buffer: Vec<u8> = vec![];
        if pipe.read_to_end(&mut buffer).is_ok() {
            if let Ok(msg) = String::from_utf8(buffer) {
                println!("{}", msg.as_str());
            }
        } else {
            println!("err")
        }
        ()
    })
}

struct WideString {
    pub source_string: String,
    bytes: Vec<u16>
}
impl WideString {
    pub fn new(str: String) -> WideString {
        let res = WideString {
            bytes: str.encode_utf16().chain(::std::iter::once(0)).collect::<Vec<u16>>(),
            source_string: str,
        };
        res
    }
    pub fn to_pcwstr(&self) -> PCWSTR {
        PCWSTR::from_raw(self.bytes.as_ptr())
    }
}

impl Display for WideString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({})", self.source_string)
    }
}

pub async fn elevate(exe_path: String, args: String, pipe_name: String) {
    println!("Try running as elevated {} {}", exe_path, args);
    let dump_task = dump_pipe(pipe_name);
    let exe_path = WideString::new(exe_path);
    let args = WideString::new(args);
    let mut exec_info: SHELLEXECUTEINFOW = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: windows::Win32::Foundation::HWND::default(),
        lpVerb: w!("runas"),
        lpFile: exe_path.to_pcwstr(),
        lpParameters: args.to_pcwstr(),
        lpDirectory: PCWSTR::null(),
        nShow: windows::Win32::UI::WindowsAndMessaging::SW_HIDE.0,
        hInstApp: Default::default(),
        lpIDList: null_mut(),
        lpClass: PCWSTR::null(),
        hkeyClass: Default::default(),
        dwHotKey: 0,
        Anonymous: Default::default(),
        hProcess: Default::default(),
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut exec_info) } {
        println!("Failed to execute as admin: {}", e.message().to_string());
        return;
    }
    let wait_result = unsafe { WaitForSingleObject(exec_info.hProcess, INFINITE) };
    if wait_result == windows::Win32::Foundation::WAIT_OBJECT_0 {
        println!("Process completed.");
        dump_task.join().unwrap();
    } else {
        println!("Failed to wait for process completion.");
    }
}

#[tokio::test]
async fn elevate_test() {
    elevate(r"F:\Rust\admin\target\debug\admin.exe".to_string(),  "aaaa aa".to_string(), "test".to_string()).await;
}
#[test]
fn elevate_tes1() {
    unsafe {
        let exe_path_original = r"F:\Rust\admin\target\debug\admin.exe".to_string();
        let args_original = "aaaa aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa aaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let exe_path = WideString::new(exe_path_original.clone());
        let args = WideString::new(args_original.clone());
        assert_eq!(exe_path_original, exe_path.to_pcwstr().display().to_string());
        assert_eq!(args_original, args.to_pcwstr().display().to_string());
    }
}
