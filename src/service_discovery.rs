use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tokio::time::{sleep, Duration};

use home_mng::Coap;

struct ProxyEntry {
    update_timestamp: SystemTime,
    service_type: Option<String>,
    address: SocketAddr,
}

pub struct Proxy {
    services: Arc<Mutex<HashMap<String, ProxyEntry>>>,
}

impl Proxy {
    const DISCOVERY_PERIOD: Duration = Duration::from_secs(600);
    const CLEANUP_PERIOD: Duration = Self::DISCOVERY_PERIOD;
    const CLEANUP_INITIAL_DELAY: Duration = Duration::from_secs(30);
    const CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3600);

    pub fn new() -> Self {
        Self {
            services: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn run(&'static self) {
        tokio::spawn(self.discovery_thread());
        tokio::spawn(self.cleanup_thread());
    }

    async fn discovery_thread(&self) {
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

            sleep(Self::DISCOVERY_PERIOD).await;
        }
    }

    async fn cleanup_thread(&self) {
        sleep(Self::CLEANUP_INITIAL_DELAY).await;

        loop {
            self.services.lock().unwrap().retain(
                    |_, v| v.update_timestamp.elapsed()
                            .unwrap_or(std::time::Duration::ZERO)
                            < Self::CLEANUP_TIMEOUT
                );
            sleep(Self::CLEANUP_PERIOD).await;
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
