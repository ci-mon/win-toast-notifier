mod notifier;

use std::collections::HashMap;
use std::env::current_exe;
use std::fmt::Debug;
use std::sync::Arc;
use std::error::Error;
use std::net::SocketAddr;
use hyper::{Body, Request, Response, Server, Method, StatusCode, header};
use hyper::body::{Buf};
use hyper::service::{make_service_fn, service_fn};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::notifier::{Notifier, ToastContent};
use crate::notifier::NotificationConfig;
use tokio::sync::{mpsc, oneshot};
use url::form_urlencoded;
use atoi::atoi;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use lazy_static::lazy_static;

#[derive(Serialize, Deserialize)]
struct NotificationRequest {
    toastXml: String,
}

#[derive(Serialize, Deserialize)]
struct NotificationResponse {
    notificationId: u32,
}

async fn notify(req: Request<Body>, push_notification: mpsc::Sender<NotificationMessage>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    let buffer = hyper::body::aggregate(req).await?;
    let request: NotificationRequest = serde_json::from_reader(Buf::reader(buffer))?;
    let config = NotificationConfig {
        content: ToastContent::Raw(request.toastXml)
    };
    let (reply_sender, mut reply_receiver) = mpsc::channel(1);
    let message = NotificationMessage::CreateNotificationRequest(config, reply_sender);
    push_notification.send(message).await.unwrap();
    let id = reply_receiver.recv().await.unwrap();
    match id {
        Ok(id_value) => {
            let response_body = json!({
                "id": id
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

async fn handler(req: Request<Body>, notifications_pipe: Sender<NotificationMessage>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => {
            let response = Response::new(Body::from("POST /notification"));
            Ok(response)
        }
        (&Method::POST, "/notification") => notify(req, notifications_pipe).await,
        (&Method::DELETE, "/notification") => hide_notification(req, notifications_pipe).await,
        (_, "/quit") => {
            return match SHUTDOWN_TX.lock().await.take().map(|x| x.send(())) {
                Some(Ok(_)) => {
                    Ok(Response::new(Body::from("Shutting down")))
                }
                _ => {
                    Ok(Response::builder().status(StatusCode::BAD_REQUEST).body(Body::empty()).unwrap())
                }
            }
        },
        _ => {
            let mut not_found = Response::default();
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

async fn hide_notification(req: Request<Body>, notifications_pipe: Sender<NotificationMessage>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    if let Some(q) = req.uri().query() {
        let params = form_urlencoded::parse(q.as_bytes())
            .into_owned()
            .collect::<HashMap<String, String>>();
        if let Some(id_str) = params.get("id") {
            if let Some(id) = atoi::<u8>(id_str.as_bytes()) {
                let (reply_sender, mut reply_receiver) = mpsc::channel(1);
                let msg = NotificationMessage::HideNotificationRequest(id, reply_sender);
                if let Ok(_) = notifications_pipe.send(msg).await {
                    return match reply_receiver.recv().await.unwrap() {
                        Ok(_) => {
                            Ok(Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap())
                        }
                        Err(e) => {
                            Ok(Response::builder().status(StatusCode::NOT_FOUND).body(Body::from(e)).unwrap())
                        }
                    };
                }
            }
        }
    }
    return Ok(Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap());
}

enum NotificationMessage {
    CreateNotificationRequest(NotificationConfig, mpsc::Sender<Result<u8, String>>),
    HideNotificationRequest(u8, mpsc::Sender<Result<(), String>>),
}

lazy_static! {
    static ref SHUTDOWN_TX: Arc<Mutex<Option<oneshot::Sender<()>>>> = <_>::default();
}
#[tokio::main]
async fn main() {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    SHUTDOWN_TX.lock().await.replace(tx);

    let application_id = if cfg!(debug_assertions) {
        "F:\\Rust\\test_toast\\target\\debug\\deps\\test_toast.exe".to_string()
    } else {
        current_exe().unwrap().as_path().display().to_string()
    };
    println!("Path={}", application_id);
    let addr = SocketAddr::from(([127, 0, 0, 1], 7070));
    let notifier = Notifier::new(&application_id).expect("Could not create notifier");
    let (sender, receiver) = mpsc::channel::<NotificationMessage>(32);
    let processing_task = tokio::spawn(async move {
        process_notification_api_messages(notifier, receiver).await;
    });

    let make_svc = make_service_fn(move |_conn| {

        let sender = sender.clone();
        async move {
            Ok::<_, Box<dyn Error + Send + Sync>>(service_fn(move |req: Request<Body>| {
                handler(req, sender.clone())
            }))
        }
    });
    let server = Server::bind(&addr).serve(make_svc);
    let info = json!({
        "ip": server.local_addr().ip().to_string(),
        "port": server.local_addr().port(),
        "application_id": application_id
    });
    println!("{}", info.to_string());
    let graceful = server.with_graceful_shutdown(async {
        rx.await.ok();
    });
    graceful.await.expect("Some error on shutdown");
    processing_task.await.unwrap();
}

async fn process_notification_api_messages(mut notifier: Notifier, mut receiver: Receiver<NotificationMessage>) {
    while let Some(received_message) = receiver.recv().await {
        match received_message {
            NotificationMessage::CreateNotificationRequest(config, respond) => {
                let id = notifier.notify(config);
                respond.send(id).await.unwrap();
            }
            NotificationMessage::HideNotificationRequest(id, respond) => {
                let result = notifier.hide(id);
                respond.send(result).await.unwrap();
            }
        }
    }
}

