use serde::Serialize;
use tokio::sync::broadcast;

use crate::tasks::Task;

#[derive(Debug, Clone, Serialize)]
pub struct AppEvent {
    pub kind: String,
    pub workspace_id: String,
    pub task: Option<Task>,
}

#[derive(Debug, Clone)]
pub struct EventBus {
    sender: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, event: AppEvent) {
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publishes_to_subscribers() {
        let bus = EventBus::new(8);
        let mut receiver = bus.subscribe();
        let event = AppEvent {
            kind: "task.created".to_owned(),
            workspace_id: "workspace".to_owned(),
            task: None,
        };

        bus.publish(event.clone());

        assert_eq!(receiver.recv().await.unwrap().kind, event.kind);
    }
}
