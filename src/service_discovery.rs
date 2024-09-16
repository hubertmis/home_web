use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tokio::time::{sleep, Duration};

use home_mng::Coap;

struct ProxyEntry {
    #[allow(dead_code)] // TODO: implement periodic removal of outdated items
    update_timestamp: SystemTime,
    service_type: Option<String>,
    address: SocketAddr,
}

pub struct Proxy {
    services: Arc<Mutex<HashMap<String, ProxyEntry>>>,
}

impl Proxy {
    pub fn new() -> Self {
        Self {
            services: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn run(&self) {
        let coap = Coap::new();

        loop {
            let services = coap.service_discovery(None, None).await.unwrap();
            
            for service in services {
                self.services.lock().unwrap().insert(service.0, ProxyEntry {
                    update_timestamp: SystemTime::now(),
                    service_type: service.1,
                    address: service.2,
                });
            }

            sleep(Duration::from_secs(600)).await;
        }
    }

    pub fn all(&self) -> Vec<(String, Option<String>, SocketAddr)> {
        self.services
            .lock()
            .unwrap()
            .iter()
            .map(|(n, e)| (n.clone(), e.service_type.clone(), e.address))
            .collect()
    }

    pub fn service(&self, name: &str) -> Option<(Option<String>, SocketAddr)> {
        self.services
            .lock()
            .unwrap()
            .get(name)
            .map(|e| (e.service_type.clone(), e.address))
    }
}
