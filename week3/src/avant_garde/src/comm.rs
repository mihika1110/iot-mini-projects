use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;
use tokio::time;
use crate::sensor::SensorData;
use crate::agent::AgentStore;

pub struct Comm {
    name: String,
    address: String,
    port: u16,
}

impl Comm {
    pub fn new(name: String, address: String, port: u16) -> Self {
        Comm {
            name: name,
            address: address,
            port: port,
        }
    }

    pub async fn run(self, agent_store: Arc<Mutex<AgentStore>>) {
        let mut mqttoptions = MqttOptions::new(self.name, self.address, self.port);
        mqttoptions.set_keep_alive(Duration::from_secs(5));
        let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
        client
            .subscribe("+/send", QoS::AtLeastOnce)
            .await
            .expect("Failed to subscribe");
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::Publish(publish))) =>  {
                    let topic = publish.topic;
                    let payload = publish.payload;

                    match serde_json::from_slice::<SensorData>(&payload) {
                        Ok(data) => {
                            let mut store = agent_store.lock().await;
                            store.update_agent(topic.as_str(), Some(data));
                            crate::logger::info(&format!("Received data from {}", topic));
                        }
                        Err(_) => {
                            let mut store = agent_store.lock().await;
                            store.update_agent(topic.as_str(), None);
                            crate::logger::warn(&format!("Received invalid data from {}", topic));
                        }
                    }
                }
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    crate::logger::info("MQTT Connected");
                }
                Ok(_) => {} // ignore other events
                Err(e) => {
                    crate::logger::error(&format!("MQTT Error: {:?}", e));
                    time::sleep(Duration::from_secs(5)).await;
                }
            }
            let mut store = agent_store.lock().await;
            store.update_expiration();
        }
    }
}
