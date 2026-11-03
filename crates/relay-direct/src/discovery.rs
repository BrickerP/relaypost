use crate::wire;
use anyhow::Result;
use mdns_sd::{DaemonEvent, Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::net::SocketAddrV4;
use std::time::Duration;

pub(crate) const SERVICE_TYPE: &str = "_relaypost._tcp.local.";

pub(crate) enum Event {
    Resolved {
        service: String,
        addresses: Vec<SocketAddrV4>,
    },
    Removed(String),
    Error(String),
}

pub(crate) struct Discovery {
    daemon: ServiceDaemon,
    events: Receiver<ServiceEvent>,
    monitor: Receiver<DaemonEvent>,
    info: ServiceInfo,
    registered: bool,
}

impl Discovery {
    pub fn start(port: u16, instance: &str, visible: bool) -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        // Any construction failure must stop the library's independently owned
        // daemon thread too; ServiceDaemon itself has no shutdown-on-drop.
        let setup = (|| -> Result<_> {
            let events = daemon.browse(SERVICE_TYPE)?;
            let monitor = daemon.monitor()?;
            let hostname = format!("relaypost-{instance}.local.");
            let info = ServiceInfo::new(
                SERVICE_TYPE,
                instance,
                &hostname,
                "",
                port,
                &[("v", "1"), ("sid", instance)][..],
            )?
            .enable_addr_auto();
            if visible {
                daemon.register(info.clone())?;
            }
            Ok((events, monitor, info))
        })();
        match setup {
            Ok((events, monitor, info)) => Ok(Self {
                daemon,
                events,
                monitor,
                info,
                registered: visible,
            }),
            Err(error) => {
                if let Ok(done) = daemon.shutdown() {
                    let _ = done.recv_timeout(Duration::from_secs(1));
                }
                Err(error)
            }
        }
    }

    pub fn visible(&mut self, visible: bool) -> Result<()> {
        if visible != self.registered {
            if visible {
                self.daemon.register(self.info.clone())?;
            } else {
                self.daemon.unregister(self.info.get_fullname())?;
            }
            self.registered = visible;
        }
        Ok(())
    }

    pub fn refresh(&self) -> Result<()> {
        self.daemon.stop_browse(SERVICE_TYPE)?;
        // browse() supplies a new stream; the existing receiver is replaced by
        // refresh_events below to avoid silently reading a stopped stream.
        Ok(())
    }

    pub fn refresh_events(&mut self) -> Result<()> {
        self.refresh()?;
        self.events = self.daemon.browse(SERVICE_TYPE)?;
        Ok(())
    }

    pub fn drain(&self) -> Vec<Event> {
        let mut output = Vec::new();
        for event in self.monitor.try_iter().take(32) {
            if let DaemonEvent::Error(error) = event {
                output.push(Event::Error(error.to_string()));
            }
        }
        for event in self.events.try_iter().take(64) {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    if info.get_property_val_str("v") != Some("1") {
                        continue;
                    }
                    let mut addresses: Vec<_> = info
                        .get_addresses_v4()
                        .into_iter()
                        .filter(|ip| wire::is_local(*ip))
                        .map(|ip| SocketAddrV4::new(ip, info.get_port()))
                        .collect();
                    addresses.sort();
                    addresses.dedup();
                    addresses.truncate(8);
                    if !addresses.is_empty() {
                        output.push(Event::Resolved {
                            service: info.get_fullname().to_owned(),
                            addresses,
                        });
                    }
                }
                ServiceEvent::ServiceRemoved(_, name) => output.push(Event::Removed(name)),
                _ => {}
            }
        }
        output
    }

    pub fn stop(&mut self) {
        if self.registered {
            let _ = self.daemon.unregister(self.info.get_fullname());
        }
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(Duration::from_millis(500));
        }
        self.registered = false;
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_record_exposes_no_identity_or_file_metadata() {
        let record = ServiceInfo::new(
            SERVICE_TYPE,
            "session-123",
            "session-123.local.",
            "",
            4567,
            &[("v", "1"), ("sid", "session-123")][..],
        )
        .unwrap()
        .enable_addr_auto();
        assert_eq!(record.get_property_val_str("v"), Some("1"));
        for private in ["name", "fingerprint", "files", "password"] {
            assert!(record.get_property_val_str(private).is_none());
        }
    }
}
