extern crate winapi;

use std::ptr::null_mut;
use winapi::um::shellapi::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS};
use winapi::um::winbase::{INFINITE, WAIT_OBJECT_0};
use winapi::um::synchapi::WaitForSingleObject;
use winapi::shared::minwindef::{DWORD, FALSE, HINSTANCE};
use std::env;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub fn elevate(args: String) {
    let exe_path = env::current_exe().expect("Failed to get current executable path");
    let exe_path_wide: Vec<u16> = OsStr::new(&exe_path)
        .encode_wide()
        .chain(Some(0).into_iter())
        .collect();
    let mut exec_info: SHELLEXECUTEINFOW = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: null_mut(),
        lpVerb: OsStr::new("runas").encode_wide().chain(Some(0).into_iter()).collect::<Vec<u16>>().as_ptr(),
        lpFile: exe_path_wide.as_ptr(),
        lpParameters: OsStr::new(args.as_str()).encode_wide().chain(Some(0).into_iter()).collect::<Vec<u16>>().as_ptr(),
        lpDirectory: null_mut(),
        nShow: winapi::um::winuser::SW_NORMAL,
        hInstApp: null_mut(),
        lpIDList: null_mut(),
        lpClass: null_mut(),
        hkeyClass: null_mut(),
        dwHotKey: 0,
        hMonitor: null_mut(),
        hProcess: null_mut(),
    };
    if unsafe { ShellExecuteExW(&mut exec_info) } == FALSE {
        println!("Failed to execute as admin.");
    } else {
        // Wait for the process to complete
        let wait_result = unsafe { WaitForSingleObject(exec_info.hProcess, INFINITE) };
        if wait_result == WAIT_OBJECT_0 {
            println!("Process completed successfully.");
        } else {
            println!("Failed to wait for process completion.");
        }
    }
}