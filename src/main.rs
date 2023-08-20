#![allow(unused_imports)]

mod notifier;
mod registerer;
mod elevator;
mod elevator_values;
mod utils;

use std::collections::HashMap;
use std::env;
use std::env::current_exe;
use std::fmt::{Debug, format};
use std::sync::{Arc, RwLock};
use std::error::Error;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::path::PathBuf;
use std::time::Duration;
use hyper::{Body, Request, Response, Server, Method, StatusCode, header};
use hyper::body::{Buf};
use hyper::service::{make_service_fn, service_fn};
use rand::{Rng, distributions::Alphanumeric};
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::notifier::{Notifier, ToastContent};
use crate::notifier::NotificationConfig;
use tokio::sync::{mpsc, oneshot};
use url::form_urlencoded;
use atoi::atoi;
use clap::builder::Str;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use clap::{Parser, Subcommand};
use tokio::fs;
use tokio::time::sleep;
use url::form_urlencoded::parse;
use winreg::enums::*;
use winreg::RegKey;
use registerer::{register_app_id, run_elevated, register_app_id_fallback, unregister_app_id};
use crate::elevator::elevate;
use crate::registerer::RegistrationError;
use crate::elevator::println_pipe;


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

async fn notify(req: Request<Body>, push_notification: Sender<WorkerMessage>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    let buffer = hyper::body::aggregate(req).await?;
    let request: NotificationRequest = serde_json::from_reader(Buf::reader(buffer))?;
    let content = get_notification_content(request);
    let config = NotificationConfig {
        content: content.expect("required field not defined")
    };
    let (reply_sender, mut reply_receiver) = mpsc::channel(1);
    let message = WorkerMessage::CreateNotificationRequest(config, reply_sender);
    push_notification.send(message).await.unwrap();
    let id = reply_receiver.recv().await.unwrap();
    match id {
        Ok(id_value) => {
            let response_body = json!({
                "id": id_value
            });
            let response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(response_body.to_string())).unwrap();
            Ok(response)
        }
        Err(error) => {
            let response = Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(error)).unwrap();
            Ok(response)
        }
    }
}

fn get_notification_content(request: NotificationRequest) -> Option<ToastContent> {
    let content = request.toast_xml_path
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .map(|x| ToastContent::Path(x))
        .or(request.toast_xml.map(|x| ToastContent::Raw(x)));
    content
}

async fn handler(req: Request<Body>, notifications_pipe: Sender<WorkerMessage>, s_sender: tokio::sync::broadcast::Sender<NotificationStatus>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    if let false = is_authorized(&req) {
        Ok(Response::builder().status(StatusCode::UNAUTHORIZED).body(Body::empty()).unwrap())
    } else {
        match (req.method(), req.uri().path()) {
            (&Method::GET, "/") => {
                let response = Response::new(Body::from("POST /notification"));
                Ok(response)
            }
            (&Method::POST, "/notification") => notify(req, notifications_pipe).await,
            (&Method::GET, "/status") => get_status(req, s_sender).await,
            (&Method::DELETE, "/notification") => hide_notification(req, notifications_pipe).await,
            (&Method::DELETE, "/all-notifications") => hide_all_notification(notifications_pipe).await,
            (_, "/quit") => {
                match SHUTDOWN_TX.lock().await.take().map(|x| x.send(())) {
                    Some(Ok(_)) => {
                        Ok(Response::new(Body::from("Shutting down")))
                    }
                    _ => {
                        Ok(Response::builder().status(StatusCode::BAD_REQUEST).body(Body::empty()).unwrap())
                    }
                }
            }
            _ => {
                let mut not_found = Response::default();
                *not_found.status_mut() = StatusCode::NOT_FOUND;
                Ok(not_found)
            }
        }
    }
}

fn is_authorized(req: &Request<Body>) -> bool {
    if let Ok(guard) = API_KEY.read() {
        if let Some(true) = req.headers().get("Api-Key").zip(guard.as_deref())
            .map(|(actual, expected)| actual.as_bytes().eq(expected))
        {
            return true;
        }
    }
    false
}

async fn hide_notification(req: Request<Body>, notifications_pipe: Sender<WorkerMessage>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    if let Some(q) = req.uri().query() {
        let params = form_urlencoded::parse(q.as_bytes())
            .into_owned()
            .collect::<HashMap<String, String>>();
        if let Some(id_str) = params.get("id") {
            if let Some(id) = atoi::<u8>(id_str.as_bytes()) {
                return Ok(send_worker_request(notifications_pipe,
                                              |reply| WorkerMessage::HideNotificationRequest(id, reply)).await);
            }
        }
    }
    return Ok(Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap());
}

async fn hide_all_notification(notifications_pipe: Sender<WorkerMessage>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    Ok(send_worker_request(notifications_pipe, |reply| WorkerMessage::HideAllNotifications(reply)).await)
}

async fn get_status(_req: Request<Body>, s_sender: tokio::sync::broadcast::Sender<NotificationStatus>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    let (mut body_tx, body) = Body::channel();
    tokio::spawn(async move {
        let mut s_rx = s_sender.subscribe();
        loop {
            match s_rx.recv().await {
                Ok(status) => {
                    let message: String = match status {
                        NotificationStatus::Activated(id, info) => {
                            json!({
                                "id": id,
                                "info": info
                            }).to_string()
                        }
                        NotificationStatus::Dismissed(id, reason) => {
                            json!({
                                "id": id,
                                "reason": reason,
                            }).to_string()
                        }
                        NotificationStatus::DismissedError(id, msg) => {
                            json!({
                                "id": id,
                                "desc": msg
                            }).to_string()
                        }
                        NotificationStatus::Failed(id, msg) => {
                            json!({
                                "id": id,
                                "desc": msg
                            }).to_string()
                        }
                    };
                    if body_tx.send_data(hyper::body::Bytes::from(message + "\n")).await.is_err() {
                        break;
                    }
                }
                _ => break
            }
        }
    });
    Ok(Response::new(body))
}

async fn send_worker_request<TMessage, Factory>(worker_pipe: Sender<TMessage>, f: Factory) -> Response<Body>
    where Factory: Fn(Sender<Result<(), String>>) -> TMessage {
    let (reply_sender, mut reply_receiver) = mpsc::channel::<Result<(), String>>(1);
    let msg = f(reply_sender);
    if let Ok(_) = worker_pipe.send(msg).await {
        return match reply_receiver.recv().await.unwrap() {
            Ok(_) => {
                Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap()
            }
            Err(e) => {
                Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(Body::from(e)).unwrap()
            }
        };
    }
    Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(Body::empty()).unwrap()
}

enum WorkerMessage {
    CreateNotificationRequest(NotificationConfig, Sender<Result<u8, String>>),
    HideNotificationRequest(u8, Sender<Result<(), String>>),
    HideAllNotifications(Sender<Result<(), String>>),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NotificationActivationInfo {
    arguments: String,
    actions: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum DismissReason {
    UserCanceled,
    ApplicationHidden,
    TimedOut
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum NotificationStatus {
    Activated(u8, NotificationActivationInfo),
    Dismissed(u8, DismissReason),
    DismissedError(u8, String),
    Failed(u8, String),
}

async fn process_notification_api_messages(mut notifier: Notifier, mut receiver: Receiver<WorkerMessage>) {
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
        #[arg(short = 't', long, )]
        title: String,
        /// Message.
        #[arg(short = 'm', long, )]
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
        #[arg(long, )]
        xml: String
    }
}
#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Registers application_id in registry. Requires admin rights.
    Register {
        /// Application Id. Example: com.app-name.module-name. See https://learn.microsoft.com/en-us/windows/win32/shell/appids
        #[arg(short = 'a', long, )]
        application_id: String,
        /// Application display name (notification header)
        #[arg(short = 'n', long)]
        display_name: Option<String>,
        /// Application icon URI (notification icon)
        #[arg(short = 'i', long)]
        icon_uri: Option<String>,
        /// Output pipe name
        #[arg(short = 'p', long)]
        parent_pipe: Option<String>,
    },
    /// Removes application_id registration in registry.
    UnRegister {
        /// Application Id.
        #[arg(short = 'a', long, )]
        application_id: String,
        /// Output pipe name
        #[arg(short = 'p', long)]
        parent_pipe: Option<String>,
    },
    /// Creates sample notification.
    Test {
        /// Application Id.
        #[arg(short = 'a', long, )]
        application_id: String,
        /// Wait.
        #[arg(long)]
        wait: bool,
        // Type
        #[command(subcommand)]
        test_type: TestType
    },
    /// Starts HTTP API.
    Run {
        /// Application Id. Can be path to executable. See https://learn.microsoft.com/en-us/windows/win32/shell/appids
        #[arg(short = 'a', long, )]
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


#[tokio::main]
async fn main() {
    let args = Args::parse();
    match args.command {
        Commands::Register { application_id, display_name, icon_uri, parent_pipe } => {
            if let Some(pipe_name) = &parent_pipe {
                elevator::enable_pipe_output(pipe_name.to_string());
                println_pipe!("Started as elevated");
            }
            match register_app_id(application_id.clone(), display_name.clone(), icon_uri.clone()) {
                Ok(_) => {
                    println_pipe!("Done");
                }
                Err(err) => {
                    match err {
                        RegistrationError::FileError(e, file) => {
                            if parent_pipe.is_some() {
                                println_pipe!("{} {}", e.to_string(), file);
                                panic!("Error: {} for {}", e.to_string(), file)
                            } else {
                                println!("Failed to register: {}", e.to_string());
                                run_elevated("register", application_id, display_name, icon_uri).await
                            }
                        }
                        RegistrationError::ArgumentError(msg) => {
                            println_pipe!("{}", msg);
                        }
                    }
                }
            };
        }
        Commands::UnRegister { application_id, parent_pipe } => {
            if let Some(pipe_name) = &parent_pipe {
                elevator::enable_pipe_output(pipe_name.to_string());
                println_pipe!("Started as elevated");
            }
            if let Err(RegistrationError::FileError(e, _f)) = unregister_app_id(application_id.clone()) {
                if parent_pipe.is_some(){
                    println!("Failed to unregister: {}", e.to_string());
                } else {
                    run_elevated("un-register", application_id, None, None).await
                }
            }
        }
        Commands::Run {
            application_id, api_key, port, ip
        } => {
            run(application_id, api_key, port, ip).await;
        }
        Commands::Test {application_id, wait, test_type} => {
            let (s_sender, _s_receiver) = tokio::sync::broadcast::channel::<NotificationStatus>(100);
            let mut  notifier = Notifier::new(&application_id, s_sender.clone()).expect("Could not create notifier");
            let content = match test_type {
                TestType::Simple { title, message, buttons, debug  } => {
                    let string = utils::create_sample_notification(title.as_str(), message.as_str(), buttons);
                    if debug {
                        println!("xml:");
                        println!("{}", string);
                    }
                    ToastContent::Raw(string)
                }
                TestType::Raw {xml} => ToastContent::Raw(xml)
            };
            register_app_id_fallback(&application_id).unwrap();
            notifier.notify(NotificationConfig {
                content
            }).expect("something was wrong");
            if wait {
                if let Ok(res) = s_sender.subscribe().recv().await {
                    println!("{}", json!(res).to_string());
                }
            } else {
                sleep(Duration::from_secs(1)).await
            }
        }
    }
}


async fn run(application_id: Option<String>, api_key: Option<String>, port: u16, ip: String) {
    let application_id = match application_id {
        None => current_exe().unwrap().file_stem().unwrap().to_str().unwrap().to_string(),
        Some(id) => id.to_string()
    };
    register_app_id_fallback(&application_id).unwrap();
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
    let (s_sender, _s_receiver) = tokio::sync::broadcast::channel::<NotificationStatus>(100);
    let notifier = Notifier::new(&application_id, s_sender.clone()).expect("Could not create notifier");
    let processing_task = tokio::spawn(async move {
        process_notification_api_messages(notifier, w_receiver).await;
    });
    let make_svc = make_service_fn(move |_conn| {
        let w_sender = w_sender.clone();
        let s_sender = s_sender.clone();
        async move {
            Ok::<_, Box<dyn Error + Send + Sync>>(service_fn(move |req: Request<Body>| {
                handler(req, w_sender.clone(), s_sender.clone())
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

#[test]
fn test() {
    let mut info = NotificationActivationInfo {
        arguments: "aaa".to_string(),
        actions: HashMap::new(),
    };
    info.actions.insert("a".to_string(), "b".to_string());
    let s = json!({
        "info": info
    }).to_string();
    println!("{}", s);
}