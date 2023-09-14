use crate::{elevator, println_pipe, utils};
use clap::builder::Str;
use serde::de::Error;
use std::env;
use std::env::current_exe;
use std::ffi::OsStr;
use std::fmt::{format, Debug, Formatter, Pointer};
use std::fs::metadata;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use winreg::enums::{HKEY_CLASSES_ROOT, KEY_ALL_ACCESS, KEY_SET_VALUE};
use winreg::RegKey;

pub async fn run_elevated(
    command: &str,
    app_id: String,
    display_name: Option<String>,
    icon_uri: Option<String>,
) -> Result<(), String> {
    let pipe_name = utils::get_random_string(20);
    let mut args = String::from(format!(
        "{} -a \"{}\" -p \"{}\"",
        command, app_id, pipe_name
    ));
    if let Some(name) = display_name {
        args.push_str(format!(" -n \"{}\"", name).as_str());
    }
    if let Some(icon) = icon_uri {
        args.push_str(format!(" -i \"{}\"", icon).as_str());
    }
    let exe_path = env::current_exe()
        .expect("Failed to get current executable path")
        .display()
        .to_string();
    elevator::elevate(exe_path, args, pipe_name).await
}

#[derive(Debug)]
pub enum RegistrationError {
    FileError(std::io::Error, String),
    ArgumentError(String),
}

pub fn unregister_app_id(app_id: String) -> Result<(), RegistrationError> {
    if app_id.contains(r"\") || app_id.contains("/") {
        return Err(RegistrationError::ArgumentError(format!(
            "app id [{}] contains invalid characters",
            app_id
        )));
    }
    let classes_root = RegKey::predef(HKEY_CLASSES_ROOT);
    if let Ok(key) = classes_root.open_subkey_with_flags("AppUserModelId", KEY_ALL_ACCESS) {
        key.delete_subkey(&app_id)
            .map_err(|e| RegistrationError::FileError(e, app_id.to_string()))?;
        println_pipe!("Unregistered");
    }
    Ok(())
}

pub fn register_app_id(
    app_id: String,
    display_name: Option<String>,
    icon_path: Option<String>,
) -> Result<(), RegistrationError> {
    if app_id.contains(r"\") || app_id.contains("/") {
        return Err(RegistrationError::ArgumentError(format!(
            "app id [{}] contains invalid characters",
            app_id
        )));
    }
    if let Some(icon) = &icon_path {
        let path = PathBuf::from(icon);
        let extension_error: String = "Icon path should have png extension.".into();
        match path
            .extension()
            .ok_or(RegistrationError::ArgumentError(extension_error.clone()))?
        {
            ext => {
                if !ext.eq("png") {
                    return Err(RegistrationError::ArgumentError(extension_error));
                }
            }
        }
    }
    let classes_root = RegKey::predef(HKEY_CLASSES_ROOT);
    let app_user_model_id = match classes_root.open_subkey("AppUserModelId") {
        Ok(key) => key,
        Err(_) => {
            let (key, _) = classes_root
                .create_subkey("AppUserModelId")
                .map_err(|e| RegistrationError::FileError(e, "AppUserModelId".to_string()))?;
            key
        }
    };
    let app_id_sub_key = match app_user_model_id.open_subkey_with_flags(&app_id, KEY_SET_VALUE) {
        Ok(key) => key,
        Err(_) => {
            let (app_id, _) = app_user_model_id
                .create_subkey(&app_id)
                .map_err(|e| RegistrationError::FileError(e, app_id.to_string()))?;
            app_id
        }
    };
    update_value(
        &app_id_sub_key,
        "DisplayName",
        display_name.or(Some(app_id.to_string())),
    )?;
    update_value(&app_id_sub_key, "IconUri", icon_path)?;
    Ok(())
}

fn update_value(key: &RegKey, name: &str, value: Option<String>) -> Result<(), RegistrationError> {
    if let Some(val) = value {
        if key.get_raw_value(name).is_ok() {
            println_pipe!("Exists");
            key.delete_value(&name)
                .map_err(|e| RegistrationError::FileError(e, format!("remove {}", name)))?;
            println_pipe!("removed");
        }
        key.set_value(&name, &val)
            .map_err(|e| RegistrationError::FileError(e, name.to_string()))?;
        println_pipe!("{} set to {}", name, val);
    }
    Ok(())
}

pub fn un_register_app_id_fallback(path: &String) -> Result<(), String> {
    let link_name = get_link_name(&path);
    let destination = get_link_path(link_name)?;
    if metadata(&destination).is_ok() {
        std::fs::remove_file(destination.clone()).map_err(|e| e.to_string())?;
        println!("Removed: {}", destination.to_str().unwrap());
    }
    Ok(())
}

fn get_link_path(link_name: String) -> Result<PathBuf, String> {
    Ok(dirs_next::home_dir()
        .ok_or("Could not find home dir")?
        .join(r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs")
        .join(format!("{link_name}.lnk")))
}

pub fn register_app_id_fallback(app_id: &String) -> Result<(), String> {
    let path = app_id.to_string();
    if !metadata(&app_id).is_ok() {
        return Ok(());
    };
    let link_name = get_link_name(&path);
    let destination = get_link_path(link_name.clone())?;
    if metadata(&destination).is_ok() {
        return Ok(());
    }
    if !sanitize_filename::is_sanitized(&link_name) {
        return Err(format!(
            "[{}] contains invalid file name characters",
            &link_name
        ));
    }
    let link = mslnk::ShellLink::new(&path).map_err(|e| e.to_string())?;
    link.create_lnk(destination.clone())
        .map_err(|e| e.to_string())?;
    println!("Registered in {}", destination.to_str().unwrap());
    Ok(())
}

fn get_link_name(path: &String) -> String {
    PathBuf::from(path.clone())
        .file_stem()
        .map(|x| x.to_str().unwrap().to_string())
        .unwrap_or(path.clone())
}
