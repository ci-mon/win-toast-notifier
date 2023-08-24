use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::error::{SendError};
use std::time::Duration;
use tokio::time::sleep;

use uuid::Uuid;
use crate::ring_buffer::RingBuffer;

#[derive(Clone)]
pub struct Sender<TEvent>
    where TEvent : Clone
{
    inner_sender: tokio::sync::mpsc::Sender<EventLogEntry<TEvent>>
}

impl<TEvent> Sender<TEvent>
    where TEvent : Clone
{

    pub fn blocking_send(&self, item: TEvent) -> Result<(), SendError<EventLogEntry<TEvent>>> {
        self.inner_sender.blocking_send(EventLogEntry::Item(item))
    }

    pub async fn subscribe(&self) -> Subscriber<TEvent> {
        let (send, recv) = tokio::sync::mpsc::channel(1000);
        let id = Uuid::new_v4();
        self.inner_sender.send(EventLogEntry::Subscribe(send, id)).await.unwrap();
        let sub = Subscriber::<TEvent> {
            _id: id,
            _inner_recv: recv,
            _inner_send: self.inner_sender.clone()
        };
        sub
    }
}
pub struct Subscriber<TEvent>
    where TEvent : Clone
{
    _id: Uuid,
    _inner_recv: tokio::sync::mpsc::Receiver<(usize, TEvent)>,
    _inner_send: tokio::sync::mpsc::Sender<EventLogEntry<TEvent>>
}

impl<TEvent> Subscriber<TEvent>
    where TEvent : Clone {

}
impl<TEvent> Subscriber<TEvent>
    where TEvent : Clone
{
    pub async fn recv(&mut self) -> Option<(usize, TEvent)> {
        self._inner_recv.recv().await
    }

    pub async fn drop_async(&mut self) {
        let id = self._id;
        let sender = self._inner_send.clone();
        let entry = EventLogEntry::UnSubscribe(id);
        if sender.send(entry).await.is_err() {
            eprintln!("Failed to unsubscribe");
        }
    }
}

pub struct Receiver<TEvent>
    where TEvent : Clone {
    inner_recv: tokio::sync::mpsc::Receiver<EventLogEntry<TEvent>>,
    buffer_len: usize,
}
impl<TEvent> Receiver<TEvent>
    where TEvent : Clone
{
    pub async fn route(&mut self) {
        let mut subscribers: Vec<(Uuid, tokio::sync::mpsc::Sender<(usize, TEvent)>)> = Vec::new();
        let mut events_buffer: RingBuffer<TEvent> = RingBuffer::new(self.buffer_len);
        loop {
            match self.inner_recv.recv().await {
                Some(EventLogEntry::Item(evt)) => {
                    let mut dead_subscribers = vec![];
                    let log_item_idx = events_buffer.get_next_number();
                    for ( index, (_, subscriber)) in subscribers.iter().enumerate() {
                        if subscriber.send((log_item_idx, evt.clone())).await.is_err() {
                            println!("Sent to dead subscriber");
                            dead_subscribers.push(index);
                        }
                    }
                    events_buffer.push(evt);
                    if !dead_subscribers.is_empty() {
                        for (number, dead_subscriber) in dead_subscribers.iter().enumerate() {
                            subscribers.remove(dead_subscriber + number);
                        }
                    }
                }
                Some(EventLogEntry::Subscribe(sender, id)) => {
                    let base_index = events_buffer.get_base_index();
                    for (idx, evt) in events_buffer.iter().enumerate() {
                        sender.send((base_index + idx, evt.clone())).await.unwrap();
                    }
                    subscribers.push((id, sender));
                }
                Some(EventLogEntry::UnSubscribe(id)) => {
                    match subscribers.iter().position(|(sub_id, _)|&id == sub_id) {
                        None => {}
                        Some(idx) => {
                            subscribers.remove(idx);
                            println!("Unsubscribed")
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug)]
pub enum EventLogEntry<TEvent> {
    Item(TEvent),
    Subscribe(tokio::sync::mpsc::Sender<(usize, TEvent)>, Uuid),
    UnSubscribe(Uuid)
}
pub fn event_log<TEvent>(buffer_len: usize) -> (Sender<TEvent>, Receiver<TEvent>)
    where TEvent : Clone {
    let (p_sender, p_recv, ) = tokio::sync::mpsc::channel::<EventLogEntry<TEvent>>(1000);
    let sender = Sender::<TEvent>{ inner_sender:  p_sender};
    let recv = Receiver::<TEvent>{ inner_recv: p_recv, buffer_len };
    (sender, recv)
}

#[tokio::test]
async  fn main_test() {
    let (s,mut r) = event_log::<String>(10);
    tokio::spawn(async move {
        r.route().await;
    });
    s.send("Hello before subs".into()).await.unwrap();
    let mut subscription = s.subscribe().await;
    tokio::spawn(async move {
        let mut counter = 3;
        while let Some((id, msg)) = subscription.recv().await {
            counter -=1;
            if counter == 0 {
                break
            }
            println!("SUB1: {} {}", id, msg)
        }
        subscription.drop_async().await;
        println!("Ended")
    });
    for i in 1..5 {
        s.send(format!("Hello after {}", i).into()).await.unwrap();
    }
    sleep(Duration::from_secs(1)).await;
    let mut subscription2 = s.subscribe().await;
    sleep(Duration::from_secs(1)).await;
    while let Ok((id, msg)) = subscription2.try_recv() {
        println!("SUB2: {} {}", id, msg)
    }
    subscription2.drop_async().await;
}