use std::collections::HashMap;
use std::fs;
use std::ops::Deref;
use rand::Rng;
use windows::Foundation::IReference;
use windows::UI::Notifications::ToastNotifier;
use windows::{
    core::{ComInterface, IInspectable, HSTRING},
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{
        ToastActivatedEventArgs, ToastDismissedEventArgs,
        ToastFailedEventArgs, ToastNotification, ToastNotificationManager,
    },
};

#[derive(Debug, Clone)]
pub enum ToastContent {
    Raw(String),
    Path(String)
}

#[derive(Debug, Clone)]
pub struct NotificationConfig {
    pub content: ToastContent
}

#[derive(Debug, Clone)]
pub struct Notification {
    id: u8,
    config: NotificationConfig,
    toast: Option<Box<ToastNotification>>
}

pub struct Notifier {
    application_id: String,
    notifications: HashMap<u8, Notification>,
    notifier: ToastNotifier
}

impl Notifier {
    pub fn new(application_id: &String) -> Result<Notifier, String> {
        match ToastNotificationManager::CreateToastNotifierWithId(&hs(application_id)) {
            Ok(notifier) => {
                Ok(Notifier {
                    notifications: HashMap::new(),
                    application_id: application_id.to_string(),
                    notifier
                })
            }
            Err(e) => Err(e.message().to_string_lossy())
        }
    }

    pub(crate) fn notify(&mut self, config: NotificationConfig) -> Result<u8,String> {
        let mut random = rand::thread_rng();
        let mut id: u8 = random.gen();
        while self.notifications.contains_key(&id) {
            id = random.gen();
        }
        let mut notification = Notification{ id, config, toast: None };
        match self.display_notification(&mut notification) {
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
    pub(crate) fn hide_all(&mut self) -> Result<(), String> {
        for notification in self.notifications.values().into_iter() {
            self.hide(&notification)?;
        }
        self.notifications.clear();
        Ok(())
    }

    pub fn hide_by_id(&mut self, id: u8) -> Result<(), String>{
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

    fn display_notification(&mut self, notification: &mut Notification) -> windows::core::Result<()> {
        let toast_doc = XmlDocument::new()?;
        let raw_content = match &notification.config.content {
            ToastContent::Raw(raw) => String::from(raw),
            ToastContent::Path(path) => fs::read_to_string(path)
                .expect("Should have been able to read the file"),
        };
        let _ = &toast_doc.LoadXml(&hs(raw_content))?;
        let toast = ToastNotification::CreateToastNotification(&toast_doc)?;
        let _ = &self.notifier.Show(&toast)?;
        toast.Activated(&TypedEventHandler::new(
            move |_, args: &Option<IInspectable>| {
                let args = args
                    .as_ref()
                    .and_then(|arg| arg.cast::<ToastActivatedEventArgs>().ok());
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
                        println!("key={}, val={}", key, val_str);
                    }
                    println!("Args={}", arguments);
                }

                Ok(())
            },
        ))?;
        toast.Dismissed(&TypedEventHandler::new(
            move |_, args: &Option<ToastDismissedEventArgs>| {
                if let Some(args) = args {
                    match args.Reason() {
                        Ok(r) => {
                            println!("Dismissed {:?}", r)
                        },
                        Err(e) =>{
                            println!("Dismissed error {:?}", e.message())
                        },
                    };
                }
                Ok(())
            },
        ))?;
        toast.Failed(&TypedEventHandler::new(
            move |_, args: &Option<ToastFailedEventArgs>| {
                if let Some(args) = args {
                    let e = args.ErrorCode().and_then(|e| e.ok());
                    if let Err(e) = e {
                        println!("Failed: {}", e.message())
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
