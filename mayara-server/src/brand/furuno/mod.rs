use async_trait::async_trait;
use log::log_enabled;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use tokio_graceful_shutdown::{SubsystemBuilder, SubsystemHandle};

// Use mayara-core for beacon parsing (pure, WASM-compatible)
use mayara_core::protocol::furuno::{
    is_beacon_response, is_model_report, parse_beacon_response, parse_model_report,
    ANNOUNCE_PACKET, REQUEST_BEACON_PACKET, REQUEST_MODEL_PACKET,
};

use crate::locator::{LocatorAddress, LocatorId, RadarLocator, RadarLocatorState};
use crate::radar::{RadarInfo, SharedRadars};
use crate::util::PrintableSlice;
use crate::{Brand, Session};

// Modules - command.rs removed, now using unified controller from mayara-core
mod data;
mod report;
mod settings;

const FURUNO_SPOKES: usize = 8192;

// Maximum supported Length of a spoke in pixels.
const FURUNO_SPOKE_LEN: usize = 883;

const FURUNO_BASE_PORT: u16 = 10000;
const FURUNO_BEACON_PORT: u16 = FURUNO_BASE_PORT + 10;
const FURUNO_DATA_PORT: u16 = FURUNO_BASE_PORT + 24;

const FURUNO_BEACON_ADDRESS: SocketAddr = SocketAddr::new(
    IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255)),
    FURUNO_BEACON_PORT,
);
const FURUNO_DATA_BROADCAST_ADDRESS: SocketAddrV4 =
    SocketAddrV4::new(Ipv4Addr::new(172, 31, 255, 255), FURUNO_DATA_PORT);

// Packet constants are now imported from mayara-core

/// Radar model enum for Furuno radars
pub(crate) enum RadarModel {
    Unknown,
    FAR21x7,
    DRS,
    FAR14x7,
    DRS4DL,
    FAR3000,
    DRS4DNXT,
    DRS6ANXT,
    DRS6AXCLASS,
    FAR15x3,
    FAR14x6,
    DRS12ANXT,
    DRS25ANXT,
}
impl RadarModel {
    /// Return the model name matching mayara-core's model database
    fn to_str(&self) -> &str {
        match self {
            RadarModel::Unknown => "Unknown",
            RadarModel::FAR21x7 => "FAR-21x7",
            RadarModel::DRS => "DRS",
            RadarModel::FAR14x7 => "FAR-14x7",
            RadarModel::DRS4DL => "DRS4DL",
            RadarModel::FAR3000 => "FAR-3000",
            RadarModel::DRS4DNXT => "DRS4D-NXT",
            RadarModel::DRS6ANXT => "DRS6A-NXT",
            RadarModel::DRS6AXCLASS => "DRS6A-XCLASS",
            RadarModel::FAR15x3 => "FAR-15x3",
            RadarModel::FAR14x6 => "FAR-14x6",
            RadarModel::DRS12ANXT => "DRS12A-NXT",
            RadarModel::DRS25ANXT => "DRS25A-NXT",
        }
    }
}

// Beacon packet structures are now in mayara-core

const LOGIN_TIMEOUT: Duration = Duration::from_millis(500);

fn login_to_radar(session: Session, radar_addr: SocketAddrV4) -> Result<u16, io::Error> {
    if session.read().unwrap().args.replay {
        log::warn!("Replay mode, not logging in to radar",);
        return Ok(0);
    }

    let mut stream =
        std::net::TcpStream::connect_timeout(&std::net::SocketAddr::V4(radar_addr), LOGIN_TIMEOUT)?;

    // fnet.dll function "login_via_copyright"
    // From the 13th byte the message is:
    // "COPYRIGHT (C) 2001 FURUNO ELECTRIC CO.,LTD. "
    const LOGIN_MESSAGE: [u8; 56] = [
        //                                              v- this byte is the only variable one
        0x8, 0x1, 0x0, 0x38, 0x1, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x43, 0x4f, 0x50, 0x59, 0x52,
        0x49, 0x47, 0x48, 0x54, 0x20, 0x28, 0x43, 0x29, 0x20, 0x32, 0x30, 0x30, 0x31, 0x20, 0x46,
        0x55, 0x52, 0x55, 0x4e, 0x4f, 0x20, 0x45, 0x4c, 0x45, 0x43, 0x54, 0x52, 0x49, 0x43, 0x20,
        0x43, 0x4f, 0x2e, 0x2c, 0x4c, 0x54, 0x44, 0x2e, 0x20,
    ];
    const EXPECTED_HEADER: [u8; 8] = [0x9, 0x1, 0x0, 0xc, 0x1, 0x0, 0x0, 0x0];

    stream.set_write_timeout(Some(LOGIN_TIMEOUT))?;
    stream.set_read_timeout(Some(LOGIN_TIMEOUT))?;

    stream.write_all(&LOGIN_MESSAGE)?;

    let mut buf: [u8; 8] = [0; 8];
    stream.read_exact(&mut buf)?;

    if buf != EXPECTED_HEADER {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Unexpected reply {:?}", buf),
        ));
    }
    stream.read_exact(&mut buf[0..4])?;

    let port = FURUNO_BASE_PORT + ((buf[0] as u16) << 8) + buf[1] as u16;
    log::debug!(
        "Furuno radar logged in; using port {} for report/command data",
        port
    );
    Ok(port)
}

#[derive(Clone)]
struct FurunoLocatorState {
    session: Session,
    radar_keys: HashMap<SocketAddrV4, String>,
    model_found: bool,
}

impl RadarLocatorState for FurunoLocatorState {
    fn process(
        &mut self,
        message: &[u8],
        from: &SocketAddrV4,
        nic_addr: &Ipv4Addr,
        radars: &SharedRadars,
        subsys: &SubsystemHandle,
    ) -> Result<(), io::Error> {
        self.process_locator_report(message, from, nic_addr, radars, subsys)
    }

    fn clone(&self) -> Box<dyn RadarLocatorState> {
        Box::new(Clone::clone(self))
    }
}

impl FurunoLocatorState {
    fn new(session: Session, radar_keys: HashMap<SocketAddrV4, String>, model_found: bool) -> Self {
        FurunoLocatorState {
            session,
            radar_keys,
            model_found,
        }
    }

    fn found(&self, info: RadarInfo, radars: &SharedRadars, subsys: &SubsystemHandle) -> bool {
        info.controls
            .set_string("userName", info.key())
            .unwrap();

        if let Some(mut info) = radars.located(info) {
            // It's new, start the RadarProcessor thread

            // Load the model name afresh, it may have been modified from persisted data
            /* let model = match info.model_name() {
                Some(s) => Model::new(&s),
                None => Model::Unknown,
            };
            if model != Model::Unknown {
                let info2 = info.clone();
                info.controls.update_when_model_known(model, &info2);
                info.set_legend(model == Model::HALO);
                radars.update(&info);
            } */

            // Furuno radars use a single TCP/IP connection to send commands and
            // receive status reports, so report_addr and send_command_addr are identical.
            // Only one of these would be enough for Furuno.
            let port: u16 = match login_to_radar(self.session.clone(), info.addr) {
                Err(e) => {
                    log::error!("{}: Unable to connect for login: {}", info.key(), e);
                    radars.remove(&info.key());
                    return false;
                }
                Ok(p) => p,
            };
            if port != info.send_command_addr.port() {
                info.send_command_addr.set_port(port);
                info.report_addr.set_port(port);
                radars.update(&info);
            }

            // Clone everything moved into future twice or more
            let data_name = info.key() + " data";
            let report_name = info.key() + " reports";

            if self.session.read().unwrap().args.output {
                let info_clone2 = info.clone();

                subsys.start(SubsystemBuilder::new("stdout", move |s| {
                    info_clone2.forward_output(s)
                }));
            }

            let data_receiver = data::FurunoDataReceiver::new(self.session.clone(), info.clone());
            subsys.start(SubsystemBuilder::new(
                data_name,
                move |s: SubsystemHandle| data_receiver.run(s),
            ));

            if !self.session.read().unwrap().args.replay {
                let report_receiver = report::FurunoReportReceiver::new(self.session.clone(), info);
                subsys.start(SubsystemBuilder::new(report_name, |s| {
                    report_receiver.run(s)
                }));
            } else {
                let model = RadarModel::DRS4DNXT; // Default model for replay
                let version = "01.05";
                log::info!(
                    "{}: Radar model {} assumed for replay mode",
                    info.key(),
                    model.to_str(),
                );
                settings::update_when_model_known(&mut info, model, version);
            }

            return true;
        }
        return false;
    }

    fn process_locator_report(
        &mut self,
        report: &[u8],
        from: &SocketAddrV4,
        via: &Ipv4Addr,
        radars: &SharedRadars,
        subsys: &SubsystemHandle,
    ) -> io::Result<()> {
        if report.len() < 2 {
            return Ok(());
        }

        if log_enabled!(log::Level::Debug) {
            log::debug!(
                "{}: Furuno report: {:02X?} len {}",
                from,
                report,
                report.len()
            );
            log::debug!("{}: printable:     {}", from, PrintableSlice::new(report));
        }

        // Use core functions to check packet type
        if is_beacon_response(report) {
            self.process_beacon_report(report, from, via, radars, subsys)
        } else if is_model_report(report) {
            self.process_beacon_model_report(report, from, via, radars)
        } else {
            Ok(())
        }
    }

    fn process_beacon_report(
        &mut self,
        report: &[u8],
        from: &SocketAddrV4,
        nic_addr: &Ipv4Addr,
        radars: &SharedRadars,
        subsys: &SubsystemHandle,
    ) -> Result<(), io::Error> {
        // Use core parsing
        let discovery = match parse_beacon_response(report, &from.to_string()) {
            Ok(d) => d,
            Err(e) => {
                log::error!(
                    "{} via {}: Failed to decode Furuno beacon: {}",
                    from,
                    nic_addr,
                    e
                );
                return Ok(());
            }
        };

        let radar_addr: SocketAddrV4 = from.clone();

        // DRS: spoke data all on a well-known address
        let spoke_data_addr: SocketAddrV4 =
            SocketAddrV4::new(Ipv4Addr::new(239, 255, 0, 2), FURUNO_DATA_PORT);

        let report_addr: SocketAddrV4 = SocketAddrV4::new(*from.ip(), 0); // Port is set in login_to_radar
        let send_command_addr: SocketAddrV4 = report_addr.clone();
        let location_info: RadarInfo = RadarInfo::new(
            self.session.clone(),
            LocatorId::Furuno,
            Brand::Furuno,
            None,
            Some(&discovery.name),
            64,
            FURUNO_SPOKES,
            FURUNO_SPOKE_LEN,
            radar_addr,
            nic_addr.clone(),
            spoke_data_addr,
            report_addr,
            send_command_addr,
            settings::new(self.session.clone()),
            true,
        );
        let key = location_info.key();
        if self.found(location_info, radars, subsys) {
            self.radar_keys.insert(from.clone(), key);
        }

        Ok(())
    }

    fn process_beacon_model_report(
        &mut self,
        report: &[u8],
        from: &SocketAddrV4,
        nic_addr: &Ipv4Addr,
        radars: &SharedRadars,
    ) -> Result<(), io::Error> {
        if self.model_found {
            return Ok(());
        }
        let radar_addr: SocketAddrV4 = from.clone();
        // Is this known as a Furuno radar?
        if let Some(key) = self.radar_keys.get(&radar_addr) {
            // Use core parsing
            match parse_model_report(report) {
                Ok((model, serial_no)) => {
                    log::debug!(
                        "{}: Furuno model report: {}",
                        from,
                        PrintableSlice::new(report)
                    );
                    log::debug!("{}: model: {:?}", from, model);
                    log::debug!("{}: serial_no: {:?}", from, serial_no);

                    if let Some(serial_no) = serial_no {
                        radars.update_serial_no(key, serial_no);
                    }

                    if model.is_some() {
                        self.model_found = true;
                    }
                }
                Err(e) => {
                    log::error!(
                        "{} via {}: Failed to decode Furuno model report: {}",
                        from,
                        nic_addr,
                        e
                    );
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
struct FurunoLocator {
    session: Session,
}

#[async_trait]
impl RadarLocator for FurunoLocator {
    fn set_listen_addresses(&self, addresses: &mut Vec<LocatorAddress>) {
        if !addresses.iter().any(|i| i.id == LocatorId::Furuno) {
            addresses.push(LocatorAddress::new(
                LocatorId::Furuno,
                &FURUNO_BEACON_ADDRESS,
                Brand::Furuno,
                vec![
                    &REQUEST_BEACON_PACKET,
                    &REQUEST_MODEL_PACKET,
                    &ANNOUNCE_PACKET,
                ],
                Box::new(FurunoLocatorState::new(
                    self.session.clone(),
                    HashMap::new(),
                    false,
                )),
            ));
        }
    }
}

pub fn create_locator(session: Session) -> Box<dyn RadarLocator + Send> {
    let locator = FurunoLocator { session };
    Box::new(locator)
}
