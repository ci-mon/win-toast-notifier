use std::fs;
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
    notifications: Vec<Notification>,
    notifier: ToastNotifier
}

impl Notifier {
    pub fn new(application_id: &String) -> Result<Notifier, String> {
        match ToastNotificationManager::CreateToastNotifierWithId(&hs(application_id)) {
            Ok(notifier) => {
                Ok(Notifier {
                    notifications: Vec::new(),
                    application_id: application_id.to_string(),
                    notifier
                })
            }
            Err(e) => Err(e.message().to_string_lossy())
        }
    }
    pub fn notify(&mut self, config: NotificationConfig) -> Result<u8,String> {
        let mut random = rand::thread_rng();
        let id: u8 = random.gen();
        let notification = Notification{ id, config, toast: None };
        match self.display_notification(&notification) {
            Ok(_) => {
                self.notifications.push(notification);
                Ok(id)
            }
            Err(error) => {
                let string = error.to_string();
                Err(string)
            }
        }
    }

    pub fn hide(&mut self, id: u8) {
        match self.notifications.iter().find(|x|x.id == id) {
            None => {}
            Some(notification) => {
                match &notification.toast  {
                    None => {}
                    Some(toast) => {
                        &self.notifier.Hide(toast.as_ref());
                    }
                }
            }
        }
    }

    fn display_notification(&mut self, n: &Notification) -> windows::core::Result<()> {
        let toast_doc = XmlDocument::new()?;
        let raw_content = match &n.config.content {
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
        Ok(())
    }
}


pub(crate) fn hs(s: impl AsRef<str>) -> HSTRING {
    HSTRING::from(s.as_ref())
}
