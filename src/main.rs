#![allow(unused_imports)]

mod elevator;
mod elevator_values;
mod event_log;
mod notifier;
mod registerer;
mod ring_buffer;
mod utils;

use crate::elevator::elevate;
use crate::elevator::println_pipe;
use crate::event_log::event_log;
use crate::notifier::NotificationConfig;
use crate::notifier::{Notifier, ToastContent};
use crate::registerer::RegistrationError;
use atoi::atoi;
use clap::builder::Str;
use clap::{Parser, Subcommand};
use hyper::body::Buf;
use hyper::service::{make_service_fn, service_fn};
use hyper::{header, Body, Method, Request, Response, Server, StatusCode};
use lazy_static::lazy_static;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::env::current_exe;
use std::error::Error;
use std::fmt::{format, Debug};
use std::fs::File;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::fs;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;
use url::form_urlencoded;
use url::form_urlencoded::parse;
use uuid::Uuid;
use winreg::enums::*;
use winreg::RegKey;

lazy_static! {
    static ref SHUTDOWN_TX: Arc<Mutex<Option<oneshot::Sender<()>>>> = <_>::default();
    static ref API_KEY: Arc<RwLock<Option<Box<[u8]>>>> = <_>::default();
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
enum TestType {
    // Create message
    Simple {
        /// Title.
        #[arg(short = 't', long)]
        title: String,
        /// Message.
        #[arg(short = 'm', long)]
        message: String,
        /// Buttons.
        #[arg(short = 'b', long)]
        buttons: Option<String>,
        /// Print xml.
        #[arg(long)]
        debug: bool,
    },
    // Raw
    Raw {
        #[arg(long)]
        xml: String,
    },
    // Raw
    RawFile {
        #[arg(long)]
        xml_path: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Registers application_id in registry. Requires admin rights.
    Register {
        /// Application Id. Example: com.app-name.module-name. See https://learn.microsoft.com/en-us/windows/win32/shell/appids
        #[arg(short = 'a', long)]
        application_id: String,
        /// Application display name (notification header)
        #[arg(short = 'n', long)]
        display_name: Option<String>,
        /// Application icon path (notification icon)
        #[arg(short = 'i', long)]
        icon_path: Option<String>,
        /// Output pipe name
        #[arg(short = 'p', long)]
        parent_pipe: Option<String>,
    },
    /// Removes application_id registration in registry.
    UnRegister {
        /// Application Id.
        #[arg(short = 'a', long)]
        application_id: String,
        /// Output pipe name
        #[arg(short = 'p', long)]
        parent_pipe: Option<String>,
    },
    /// Creates sample notification.
    Test {
        /// Application Id.
        #[arg(short = 'a', long)]
        application_id: String,
        /// Wait.
        #[arg(long)]
        wait: bool,
        // Type
        #[command(subcommand)]
        test_type: TestType,
    },
    /// Starts HTTP API.
    Listen {
        /// Application Id. Can be path to executable. See https://learn.microsoft.com/en-us/windows/win32/shell/appids
        #[arg(short = 'a', long)]
        application_id: Option<String>,
        /// HTTP API key, should be specified in api-key header
        #[arg(short = 'k', long)]
        api_key: Option<String>,
        /// TCP port to listen on
        #[arg(short, long, default_value_t = 7070)]
        port: u16,
        /// IP Address to listen on
        #[arg(short, long, default_value = "127.0.0.1")]
        ip: String,
    },
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct NotificationRequest {
    #[serde(default)]
    toast_xml: Option<String>,
    #[serde(default)]
    toast_xml_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct NotificationResponse {
    notification_id: u32,
}

enum WorkerMessage {
    CreateNotificationRequest(NotificationConfig, Sender<Result<Uuid, String>>),
    HideNotificationRequest(Uuid, Sender<Result<(), String>>),
    HideAllNotifications(Sender<Result<(), String>>),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NotificationActivationInfo {
    arguments: String,
    inputs: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum DismissReason {
    UserCanceled,
    ApplicationHidden,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum NotificationStatus {
    Activated(String, NotificationActivationInfo),
    Dismissed(String, DismissReason),
    DismissedError(String, String),
    Failed(String, String),
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    match args.command {
        Commands::Register {
            application_id,
            display_name,
            icon_path,
            parent_pipe,
        } => {
            register(application_id, display_name, icon_path, &parent_pipe).await;
        }
        Commands::UnRegister {
            application_id,
            parent_pipe,
        } => {
            un_register(application_id, &parent_pipe).await;
        }
        Commands::Listen {
            application_id,
            api_key,
            port,
            ip,
        } => {
            listen(application_id, api_key, port, ip).await;
        }
        Commands::Test {
            application_id,
            wait,
            test_type,
        } => {
            test(&application_id, wait, test_type).await;
        }
    }
}

async fn test(application_id: &String, wait: bool, test_type: TestType) {
    let (n_sender, mut n_recv) = event_log::<NotificationStatus>(1000);
    tokio::spawn(async move {
        n_recv.init_transport().await;
    });
    let mut notifier =
        Notifier::new(&application_id, n_sender.clone()).expect("Could not create notifier");
    let content = match test_type {
        TestType::Simple {
            title,
            message,
            buttons,
            debug,
        } => {
            let string =
                utils::create_sample_notification(title.as_str(), message.as_str(), buttons);
            if debug {
                println!("xml:");
                println!("{}", string);
            }
            ToastContent::Raw(string)
        }
        TestType::Raw { xml } => ToastContent::Raw(xml),
        TestType::RawFile { xml_path } => ToastContent::Raw(fs::read_to_string(xml_path).await.unwrap()),
    };
    registerer::register_app_id_fallback(&application_id).unwrap();
    notifier
        .notify(NotificationConfig { content })
        .expect("something was wrong");
    if wait {
        if let Some((num, res)) = n_sender.subscribe().await.recv().await {
            println!(
                "{}",
                json!({
                    "event_number": num,
                    "event": res
                })
                    .to_string()
            );
        }
    } else {
        sleep(Duration::from_secs(1)).await
    }
}

async fn un_register(application_id: String, parent_pipe: &Option<String>) {
    if let Some(pipe_name) = &parent_pipe {
        elevator::enable_pipe_output(pipe_name.to_string());
        println_pipe!("Started as elevated");
    }
    if std::fs::metadata(&application_id).is_ok() {
        registerer::un_register_app_id_fallback(&application_id).expect("Failed to unregister");
        return;
    }
    if let Err(RegistrationError::FileError(e, _f)) =
        registerer::unregister_app_id(application_id.clone())
    {
        if parent_pipe.is_some() {
            println!("Failed to unregister: {}", e.to_string());
        } else {
            registerer::run_elevated("un-register", application_id, None, None)
                .await
                .expect("Failed to run as admin");
        }
    }
}

async fn register(
    application_id: String,
    display_name: Option<String>,
    icon_path: Option<String>,
    parent_pipe: &Option<String>,
) {
    if let Some(pipe_name) = &parent_pipe {
        elevator::enable_pipe_output(pipe_name.to_string());
        println_pipe!("Started as elevated");
    }
    if std::fs::metadata(&application_id).is_ok() && display_name.is_none() {
        registerer::register_app_id_fallback(&application_id).expect("Failed to register");
        return;
    }
    match registerer::register_app_id(
        application_id.clone(),
        display_name.clone(),
        icon_path.clone(),
    ) {
        Ok(_) => {
            println_pipe!("Done");
        }
        Err(err) => match err {
            RegistrationError::FileError(e, file) => {
                if parent_pipe.is_some() {
                    println_pipe!("{} {}", e.to_string(), file);
                    panic!("Error: {} for {}", e.to_string(), file)
                } else {
                    println!("Failed to register: {}", e.to_string());
                    registerer::run_elevated(
                        "register",
                        application_id.clone(),
                        display_name,
                        icon_path,
                    )
                        .await
                        .expect("Cant run elevated");
                }
            }
            RegistrationError::ArgumentError(msg) => {
                println_pipe!("{}", msg);
            }
        },
    };
}

async fn listen(application_id: Option<String>, api_key: Option<String>, port: u16, ip: String) {
    let application_id = match application_id {
        None => current_exe()
            .unwrap()
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string(),
        Some(id) => id.to_string(),
    };
    registerer::register_app_id_fallback(&application_id).unwrap();
    let (tx, rx) = oneshot::channel::<()>();
    SHUTDOWN_TX.lock().await.replace(tx);
    let api_key = api_key.unwrap_or(utils::get_random_string(50));
    match API_KEY.write() {
        Ok(mut guard) => {
            guard.replace(api_key.as_bytes().to_vec().into_boxed_slice());
        }
        Err(_) => {}
    }
    let addr = SocketAddr::from((ip.parse::<Ipv4Addr>().expect("invalid ip address"), port));
    let (w_sender, w_receiver) = mpsc::channel::<WorkerMessage>(32);
    let (n_sender, mut n_recv) = event_log::<NotificationStatus>(1000);
    tokio::spawn(async move {
        n_recv.init_transport().await;
    });
    let notifier =
        Notifier::new(&application_id, n_sender.clone()).expect("Could not create notifier");
    let processing_task = tokio::spawn(async move {
        process_notification_api_messages(notifier, w_receiver).await;
    });
    let make_svc = make_service_fn(move |_conn| {
        let w_sender = w_sender.clone();
        let n_sub_factory = n_sender.clone();
        async move {
            Ok::<_, Box<dyn Error + Send + Sync>>(service_fn(move |req: Request<Body>| {
                http_handler(req, w_sender.clone(), n_sub_factory.clone())
            }))
        }
    });
    let server = Server::bind(&addr).serve(make_svc);
    let info = json!({
        "ip": server.local_addr().ip().to_string(),
        "port": server.local_addr().port(),
        "application_id": application_id,
        "api_key": api_key
    });
    println!("{}", info.to_string());
    let graceful = server.with_graceful_shutdown(async {
        rx.await.ok();
    });
    graceful.await.expect("Some error on shutdown");
    processing_task.await.unwrap();
}

async fn notify(
    req: Request<Body>,
    push_notification: Sender<WorkerMessage>,
) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    let buffer = hyper::body::aggregate(req).await?;
    let request: NotificationRequest = serde_json::from_reader(Buf::reader(buffer))?;
    let content = get_notification_content(request);
    let config = NotificationConfig {
        content: content.expect("required field not defined"),
    };
    let (reply_sender, mut reply_receiver) = mpsc::channel(1);
    let message = WorkerMessage::CreateNotificationRequest(config, reply_sender);
    push_notification.send(message).await.unwrap();
    let id = reply_receiver.recv().await.unwrap();
    match id {
        Ok(id_value) => {
            let response_body = json!({
                "id": id_value.to_string()
            });
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(response_body.to_string()))
                .unwrap();
            Ok(response)
        }
        Err(error) => {
            let response = Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(error))
                .unwrap();
            Ok(response)
        }
    }
}

fn get_notification_content(request: NotificationRequest) -> Option<ToastContent> {
    let content = request
        .toast_xml_path
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .map(|x| ToastContent::Path(x))
        .or(request.toast_xml.map(|x| ToastContent::Raw(x)));
    content
}

async fn http_handler(
    req: Request<Body>,
    notifications_pipe: Sender<WorkerMessage>,
    s_sender: event_log::Sender<NotificationStatus>,
) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    if let false = is_authorized(&req) {
        Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .unwrap())
    } else {
        match (req.method(), req.uri().path()) {
            (&Method::GET, "/") => {
                let response = Response::new(Body::from("POST /notification"));
                Ok(response)
            }
            (&Method::POST, "/notify") => notify(req, notifications_pipe).await,
            (&Method::GET, "/status-stream") => get_status(req, s_sender).await,
            (&Method::DELETE, "/notification") => hide_notification(req, notifications_pipe).await,
            (&Method::DELETE, "/all") => hide_all_notification(notifications_pipe).await,
            (_, "/quit") => match SHUTDOWN_TX.lock().await.take().map(|x| x.send(())) {
                Some(Ok(_)) => Ok(Response::new(Body::from("Shutting down"))),
                _ => Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::empty())
                    .unwrap()),
            },
            _ => {
                let not_found = Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("Endpoint not found"))
                    .unwrap();
                Ok(not_found)
            }
        }
    }
}

fn is_authorized(req: &Request<Body>) -> bool {
    if let Ok(guard) = API_KEY.read() {
        if let Some(true) = req
            .headers()
            .get("Api-Key")
            .zip(guard.as_deref())
            .map(|(actual, expected)| actual.as_bytes().eq(expected))
        {
            return true;
        }
    }
    false
}

async fn hide_notification(
    req: Request<Body>,
    notifications_pipe: Sender<WorkerMessage>,
) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    if let Some(q) = req.uri().query() {
        let params = form_urlencoded::parse(q.as_bytes())
            .into_owned()
            .collect::<HashMap<String, String>>();
        if let Some(id_str) = params.get("id") {
            if let Ok(id) = Uuid::parse_str(id_str) {
                return Ok(send_worker_request(notifications_pipe, |reply| {
                    WorkerMessage::HideNotificationRequest(id, reply)
                })
                    .await);
            }
        }
    }
    return Ok(Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::empty())
        .unwrap());
}

async fn hide_all_notification(
    notifications_pipe: Sender<WorkerMessage>,
) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    Ok(send_worker_request(notifications_pipe, |reply| {
        WorkerMessage::HideAllNotifications(reply)
    })
        .await)
}

async fn get_status(
    _req: Request<Body>,
    s_sender: event_log::Sender<NotificationStatus>,
) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    let last_number = _req
        .uri()
        .query()
        .map(|q| {
            form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect::<HashMap<String, String>>()
        })
        .and_then(|h| h.get("from").map(|x| x.to_string()))
        .and_then(|id| atoi::<usize>(id.as_bytes()))
        .unwrap_or(0);

    let (mut body_tx, body) = Body::channel();
    let mut subscriber = s_sender.subscribe().await;
    tokio::spawn(async move {
        loop {
            match subscriber.recv().await {
                Some((num, status)) => {
                    if last_number > num {
                        continue;
                    }
                    let message: String = match status {
                        NotificationStatus::Activated(id, info) => json!({
                            "number": num,
                            "id": id,
                            "info": info,
                            "type": "Activated"
                        })
                            .to_string(),
                        NotificationStatus::Dismissed(id, reason) => json!({
                            "number": num,
                            "id": id,
                            "dismissReason": reason,
                            "type": "Dismissed"
                        })
                            .to_string(),
                        NotificationStatus::DismissedError(id, msg) => json!({
                            "number": num,
                            "id": id,
                            "description": msg,
                            "type": "DismissedError"
                        })
                            .to_string(),
                        NotificationStatus::Failed(id, msg) => json!({
                            "number": num,
                            "id": id,
                            "description": msg,
                            "type": "Failed"
                        })
                            .to_string(),
                    };
                    if body_tx
                        .send_data(hyper::body::Bytes::from(message + "\n"))
                        .await
                        .is_err()
                    {
                        subscriber.drop_async().await;
                        break;
                    }
                }
                _ => break,
            }
        }
    });
    Ok(Response::new(body))
}

async fn send_worker_request<TMessage, Factory>(
    worker_pipe: Sender<TMessage>,
    f: Factory,
) -> Response<Body>
    where
        Factory: Fn(Sender<Result<(), String>>) -> TMessage,
{
    let (reply_sender, mut reply_receiver) = mpsc::channel::<Result<(), String>>(1);
    let msg = f(reply_sender);
    if let Ok(_) = worker_pipe.send(msg).await {
        return match reply_receiver.recv().await.unwrap() {
            Ok(_) => Response::builder()
                .status(StatusCode::OK)
                .body(Body::empty())
                .unwrap(),
            Err(e) => Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(e))
                .unwrap(),
        };
    }
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(Body::empty())
        .unwrap()
}

async fn process_notification_api_messages(
    mut notifier: Notifier,
    mut receiver: Receiver<WorkerMessage>,
) {
    while let Some(received_message) = receiver.recv().await {
        match received_message {
            WorkerMessage::CreateNotificationRequest(config, respond) => {
                let id = notifier.notify(config);
                respond.send(id).await.unwrap();
            }
            WorkerMessage::HideNotificationRequest(id, respond) => {
                let result = notifier.hide_by_id(id);
                respond.send(result).await.unwrap();
            }
            WorkerMessage::HideAllNotifications(respond) => {
                respond.send(notifier.hide_all()).await.unwrap();
            }
        }
    }
}
