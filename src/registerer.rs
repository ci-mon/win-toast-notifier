use std::env;
use std::ffi::OsStr;
use std::fmt::format;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use winreg::enums::HKEY_CLASSES_ROOT;
use winreg::RegKey;
use crate::elevator;

pub fn register_app_id_elevated(app_id: String, display_name: Option<String>, icon_uri: Option<String>){
    let mut args = String::from(format!("register -a \"{}\" --no-elevate", app_id));
    if let Some(name) = display_name{
        args.push_str(format!(" -n \"{}\"", name).as_str());
    }
    if let Some(icon) = icon_uri {
        args.push_str(format!(" -i \"{}\"", icon).as_str());
    }
    elevator::elevate(args);
}

pub enum RegistrationError {
    FileError(std::io::Error),
    ArgumentError(String),
}
pub fn register_app_id(app_id: String, display_name: Option<String>, icon_uri: Option<String>) -> Result<(), RegistrationError> {
    if app_id.contains(r"\") || app_id.contains("/") {
        return Err(RegistrationError::ArgumentError(format!("app id [{}] contains invalid characters", app_id)));
    }
    let classes_root = RegKey::predef(HKEY_CLASSES_ROOT);
    let app_user_model_id = match classes_root.open_subkey("AppUserModelId") {
        Ok(key) => {
            key
        }
        Err(_) => {
            let (key, _) = classes_root.create_subkey("AppUserModelId").map_err(RegistrationError::FileError)?;
            key
        }
    };
    if let Ok(_) = app_user_model_id.open_subkey(&app_id){
        return Ok(());
    }
    let (app_id, _) = app_user_model_id.create_subkey(&app_id).map_err(RegistrationError::FileError)?;
    if let Some(name) = display_name {
        app_id.set_value("DisplayName", &name).map_err(RegistrationError::FileError)?;
    }
    if let Some(icon) = icon_uri {
        app_id.set_value("IconUri", &icon).map_err(RegistrationError::FileError)?;
    }
    Ok(())
}
pub fn register_app_id_fallback(app_id: &String) -> Result<(), String>{
    if !std::fs::metadata(&app_id).is_ok() {
        return Ok(());
    };
    let link_name = PathBuf::from(app_id).file_stem().map(|x|x.to_str().unwrap().to_string()).unwrap_or(app_id.clone());
    let destination = dirs_next::home_dir().ok_or("Could not find home dir")?
        .join(r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs")
        .join(format!("{link_name}.lnk"));
    if std::fs::metadata(&destination).is_ok() {
        return Ok(());
    }
    if !sanitize_filename::is_sanitized(&link_name){
        return Err(format!("[{}] contains invalid file name characters", &link_name))
    }
    let mut link = mslnk::ShellLink::new(&app_id).map_err(|e|e.to_string())?;
    link.create_lnk(destination).map_err(|e|e.to_string())?;
    Ok(())
}