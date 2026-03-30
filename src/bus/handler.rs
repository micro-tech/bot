use super::{Bus, Message};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::utils::log_to_file;
use log::error; // needed for error!() // needed for log_to_file()

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

    pub fn publish(&self, msg: Message) -> Result<(), String> {
        self.bus.publish(msg)
    }

    pub fn publish_or_log(&self, msg: Message) {
        if let Err(e) = self.bus.publish(msg.clone()) {
            let error_msg = format!("BusHandle publish_or_log failed: {}", e);
            log_to_file(&error_msg);
            error!("{}", error_msg);
        }
    }

    pub fn try_recv(&self) -> Option<Message> {
        match self.rx.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => None,
            Err(_) => None,
        }
    }
}
