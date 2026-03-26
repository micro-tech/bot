use super::{Bus, Message};
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Clone)]
pub struct BusHandle {
    pub bus: Bus,
    pub rx: Receiver<Message>,
}

impl BusHandle {
    pub fn new(bus: Bus, component_name: &str) -> Self {
        let rx = bus.subscribe(component_name);
        Self { bus, rx }
    }

    pub fn publish(&self, msg: Message) {
        self.bus.publish(msg);
    }

    pub fn try_recv(&self) -> Option<Message> {
        match self.rx.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => None,
            Err(_) => None,
        }
    }
}
