mod notifier;

use std::env::current_exe;
use std::fmt::Debug;

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
use tokio::sync::mpsc;

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
    let message = NotificationMessage::Request(config, reply_sender);
    push_notification.send(message).await.unwrap();
    let id = reply_receiver.recv().await.unwrap();
    match id {
        Ok(id_value) => {
            let response_body = json!({
                "notification_id": id
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

async fn handler(req: Request<Body>, create_notification: mpsc::Sender<NotificationMessage>) -> Result<Response<Body>, Box<dyn Error + Send + Sync>> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => {
            let response = Response::new(Body::from("POST /notification"));
            Ok(response)
        }
        (&Method::POST, "/notification") => notify(req, create_notification).await,
        (&Method::DELETE, "/notification") => notify(req, create_notification).await,
        _ => {
            let mut not_found = Response::default();
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

enum NotificationMessage {
    Request(NotificationConfig, mpsc::Sender<Result<u8, String>>),
}

#[tokio::main]
async fn main() {
    let application_id = if cfg!(debug_assertions) {
        "F:\\Rust\\test_toast\\target\\debug\\deps\\test_toast.exe".to_string()
    } else {
        current_exe().unwrap().as_path().display().to_string()
    };
    println!("Path={}", application_id);


    let addr = SocketAddr::from(([127, 0, 0, 1], 7070));
    let mut notifier = Notifier::new(&application_id).expect("Could not create notifier");
    let (sender, mut receiver) = mpsc::channel::<NotificationMessage>(32);
    let processing_task = tokio::spawn(async move {
        while let Some(received_message) = receiver.recv().await {
            match received_message {
                NotificationMessage::Request(config, response) => {
                    let id= notifier.notify(config);
                    response.send(id).await.unwrap();
                }
            }
        }
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
    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
    processing_task.await.unwrap();

}

