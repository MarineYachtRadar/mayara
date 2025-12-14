use async_trait::async_trait;
use log::debug;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::sleep;
use tokio_shutdown::Shutdown;

use crate::locator::RadarLocator;
use crate::radar::{located, RadarLocationInfo, Radars};
use crate::util::join_multicast;

mod report;

const GARMIN_REPORT_ADDRESS: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(239, 254, 2, 0)), 50100);
const GARMIN_SEND_PORT: u16 = 50101;
const GARMIN_DATA_ADDRESS: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(239, 254, 2, 0)), 50102);

fn process_beacon(report: &[u8]) {
    if report.len() < 2 {
        return;
    }

    report::process(report);
}

struct GarminLocator {
    found: bool,
    buf: Vec<u8>,
    sock: Option<UdpSocket>,
}

impl GarminLocator {
    async fn start(&mut self) -> io::Result<()> {
        match join_multicast(&GARMIN_REPORT_ADDRESS).await {
            Ok(sock) => {
                self.sock = Some(sock);
                debug!(
                    "Listening on {} for Garmin xHD (and others?)",
                    GARMIN_REPORT_ADDRESS
                );
                Ok(())
            }
            Err(e) => {
                sleep(Duration::from_millis(1000)).await;
                debug!("Beacon multicast failed: {}", e);
                Ok(())
            }
        }
    }
}

async fn found(
    info: RadarLocationInfo,
    radars: &SharedRadars,
    shutdown: &Shutdown,
) -> io::Result<()> {
    if let Some(info) = located(info, radars) {
        // It's new, start the RadarProcessor thread
        // let shutdown = shutdown.clone();
        //tokio::spawn(async move {
        //   receive::Receive::run(info, shutdown).await.unwrap();
        //});
        // TODO do something with the join handle
    }
    Ok(())
}

#[async_trait]
impl RadarLocator for GarminLocator {
    async fn process_beacons(
        &mut self,
        radars: SharedRadars,
        shutdown: Shutdown,
    ) -> io::Result<()> {
        self.start().await.unwrap();
        loop {
            match &self.sock {
                Some(sock) => {
                    self.buf.clear();
                    match sock.recv_buf_from(&mut self.buf).await {
                        Ok((_len, from)) => {
                            if self.found {
                                process_beacon(&self.buf);
                            } else {
                                let mut radar_send = from.clone();

                                radar_send.set_port(GARMIN_SEND_PORT);
                                let location_info: RadarLocationInfo = RadarLocationInfo::new(
                                    "Garmin",
                                    None,
                                    None,
                                    None,
                                    from,
                                    GARMIN_DATA_ADDRESS,
                                    GARMIN_REPORT_ADDRESS,
                                    radar_send,
                                );
                                found(location_info, &radars, &shutdown).await.unwrap();

                                self.found = true;
                            }
                        }
                        Err(e) => {
                            debug!("Beacon read failed: {}", e);
                            self.sock = None;
                        }
                    }
                }
                None => {
                    sleep(Duration::from_millis(1000)).await;
                    self.start().await.unwrap();
                }
            }
        }
    }
}

pub fn create_locator() -> Box<dyn RadarLocator + Send> {
    let locator = GarminLocator {
        found: false,
        buf: Vec::with_capacity(2048),
        sock: None,
    };
    Box::new(locator)
}
