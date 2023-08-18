mod notifier;

use std::collections::HashMap;
use std::env::current_exe;
use std::fmt::Debug;
use std::sync::{Arc, LockResult, RwLock};
use std::error::Error;
use std::net::SocketAddr;
use std::ops::Deref;
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
use hyper::header::HeaderValue;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use tokio::sync::broadcast::error::RecvError;

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

async fn get_status(req: Request<Body>, s_sender: tokio::sync::broadcast::Sender<NotificationStatus>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    let (mut tx, body) = Body::channel();
    tokio::spawn(async move {
        let mut s_rx = s_sender.subscribe();
        while true {
            match s_rx.recv().await {
                Ok(_) => {}
                _ => break
            }
            /*let chunk = format!("{}\n", i);
            if tx.send_data(chunk.into()).is_err() {
                break;
            }*/
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

#[derive(Clone)]
enum NotificationStatus {
    Activated(u8),
    Dismissed(u8),
    Failed(u8),
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



#[tokio::main]
async fn main() {
    let (tx, rx) = oneshot::channel::<()>();
    SHUTDOWN_TX.lock().await.replace(tx);
    let api_key: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(50)
        .map(char::from)
        .collect();
    match API_KEY.write() {
        Ok(mut guard) => {
            guard.replace(api_key.as_bytes().to_vec().into_boxed_slice());
        }
        Err(_) => {}
    }
    let application_id = if cfg!(debug_assertions) {
        "F:\\Rust\\test_toast\\target\\debug\\deps\\test_toast.exe".to_string()
    } else {
        current_exe().unwrap().as_path().display().to_string()
    };
    println!("Path={}", application_id);
    let addr = SocketAddr::from(([127, 0, 0, 1], 7070));
    let notifier = Notifier::new(&application_id).expect("Could not create notifier");
    let (w_sender, w_receiver) = mpsc::channel::<WorkerMessage>(32);
    let (s_sender, s_receiver) = tokio::sync::broadcast::channel::<NotificationStatus>(100);
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
    let request = serde_json::from_str::<NotificationRequest>("{\"toast_xml\": \"1\"}").unwrap();
    assert_eq!(request, NotificationRequest { toast_xml: Some("1".to_string()), toast_xml_path: None });
    let content = get_notification_content(request);
    content.expect("content empty");
}