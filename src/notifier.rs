use std::collections::HashMap;
use std::fs;
use std::io::Error;
use std::ops::Deref;
use std::sync::Arc;
use rand::Rng;
use tokio::sync::mpsc::Sender;
use windows::Foundation::IReference;
use windows::UI::Notifications::{ToastDismissalReason, ToastNotifier};
use windows::{
    core::{ComInterface, IInspectable, HSTRING},
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{
        ToastActivatedEventArgs, ToastDismissedEventArgs,
        ToastFailedEventArgs, ToastNotification, ToastNotificationManager,
    },
};
use crate::{DismissReason, event_log, NotificationActivationInfo, NotificationStatus};

#[derive(Debug, Clone)]
pub enum ToastContent {
    Raw(String),
    Path(String),
}

#[derive(Debug, Clone)]
pub struct NotificationConfig {
    pub content: ToastContent,
}

#[derive(Debug, Clone)]
pub struct Notification {
    id: u8,
    config: NotificationConfig,
    toast: Option<Box<ToastNotification>>,
}

pub struct Notifier {
    notifications: HashMap<u8, Notification>,
    notifier: ToastNotifier,
    status_writer: event_log::Sender<NotificationStatus>,
}

impl Notifier {
    pub fn new(application_id: &String, s_sender: event_log::Sender<NotificationStatus>) -> Result<Notifier, String> {
        match ToastNotificationManager::CreateToastNotifierWithId(&hs(application_id)) {
            Ok(notifier) => {
                Ok(Notifier {
                    notifications: HashMap::new(),
                    notifier,
                    status_writer: s_sender,
                })
            }
            Err(e) => Err(e.message().to_string_lossy())
        }
    }
    pub(crate) fn notify(&mut self, config: NotificationConfig) -> Result<u8, String> {
        let mut random = rand::thread_rng();
        let mut id: u8 = random.gen();
        while self.notifications.contains_key(&id) {
            id = random.gen();
        }
        let mut notification = Notification { id, config, toast: None };
        let raw_content = match &notification.config.content {
            ToastContent::Raw(raw) => Ok(String::from(raw)),
            ToastContent::Path(path) => fs::read_to_string(path).map_err(|x| format!("{}. path={}", x.to_string(), path))
        };
        match raw_content {
            Ok(content) => {
                match self.display_notification(&mut notification, content) {
                    Ok(_) => {
                        self.notifications.insert(notification.id, notification);
                        Ok(id)
                    }
                    Err(error) => {
                        let string = error.to_string();
                        Err(string)
                    }
                }
            }
            Err(msg) => {
                Err(msg)
            }
        }
    }
    pub(crate) fn hide_all(&mut self) -> Result<(), String> {
        for notification in self.notifications.values().into_iter() {
            self.hide(&notification)?;
        }
        self.notifications.clear();
        Ok(())
    }

    pub fn hide_by_id(&mut self, id: u8) -> Result<(), String> {
        match self.notifications.remove(&id) {
            None => {
                Err("Not found".to_string())
            }
            Some(notification) => {
                self.hide(&notification)?;
                Ok(())
            }
        }
    }

    fn hide(&self, notification: &Notification) -> Result<(), String> {
        match notification.toast.as_ref() {
            None => {
                Err("Toast not defined".to_string())
            }
            Some(toast) => {
                let toast = toast.deref();
                match self.notifier.Hide(toast) {
                    Ok(_) => {
                        Ok(())
                    }
                    Err(e) => {
                        Err(e.message().to_string_lossy())
                    }
                }
            }
        }
    }

    fn display_notification(&mut self, notification: &mut Notification, raw_content: String) -> windows::core::Result<()> {
        let toast_doc = XmlDocument::new()?;
        let _ = &toast_doc.LoadXml(&hs(raw_content))?;
        let toast = ToastNotification::CreateToastNotification(&toast_doc)?;
        toast.SetExpiresOnReboot(true)?;
        let _ = &self.notifier.Show(&toast)?;
        let a_status_writer = self.status_writer.clone();
        let notification_id = notification.id;
        toast.Activated(&TypedEventHandler::new(
            move |_, args: &Option<IInspectable>| {
                let args = args
                    .as_ref()
                    .and_then(|arg| arg.cast::<ToastActivatedEventArgs>().ok());
                let mut actions = HashMap::<String, String>::new();
                if let Some(args) = args {
                    let arguments = args.Arguments().map(|s| s.to_string_lossy()).unwrap();
                    let user_input = args.UserInput()?;
                    for el in user_input.into_iter() {
                        let key = el.Key()?.to_string_lossy();
                        let value = el.Value()?;
                        let val_str = value
                            .cast::<IReference<HSTRING>>()?
                            .GetString()?
                            .to_string_lossy();
                        actions.insert(key, val_str);
                    }
                    let info = NotificationActivationInfo {
                        arguments,
                        actions,
                    };
                    let status = NotificationStatus::Activated(notification_id, info);
                    a_status_writer.blocking_send(status).ok();
                }
                Ok(())
            },
        ))?;
        let d_status_writer = self.status_writer.clone();
        toast.Dismissed(&TypedEventHandler::new(move |_, args: &Option<ToastDismissedEventArgs>| {
            if let Some(args) = args {
                let status = match args.Reason() {
                    Ok(reason) => {
                        let reason = match reason {
                            ToastDismissalReason::UserCanceled => DismissReason::UserCanceled,
                            ToastDismissalReason::ApplicationHidden => DismissReason::ApplicationHidden,
                            _ => DismissReason::TimedOut,
                        };
                        NotificationStatus::Dismissed(notification_id, reason)
                    }
                    Err(e) => {
                        NotificationStatus::DismissedError(notification_id, e.message().to_string())
                    }
                };
                d_status_writer.blocking_send(status).ok();
            }
            Ok(())
        }, ))?;
        let f_status_writer = self.status_writer.clone();
        toast.Failed(&TypedEventHandler::new(
            move |_, args: &Option<ToastFailedEventArgs>| {
                if let Some(args) = args {
                    let e = args.ErrorCode().and_then(|e| e.ok());
                    if let Err(e) = e {
                        let status = NotificationStatus::Failed(notification_id, e.message().to_string());
                        f_status_writer.blocking_send(status).ok();
                    }
                }
                Ok(())
            },
        ))?;
        notification.toast = Some(Box::new(toast));
        Ok(())
    }
}


pub(crate) fn hs(s: impl AsRef<str>) -> HSTRING {
    HSTRING::from(s.as_ref())
}
