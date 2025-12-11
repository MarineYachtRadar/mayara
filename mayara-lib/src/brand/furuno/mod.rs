#[allow(dead_code)]
use async_trait::async_trait;
use bincode::deserialize;
use log::log_enabled;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use tokio_graceful_shutdown::{SubsystemBuilder, SubsystemHandle};

use crate::locator::{LocatorAddress, LocatorId, RadarLocator, RadarLocatorState};
use crate::radar::{RadarInfo, SharedRadars};
use crate::util::{c_string, PrintableSlice};
use crate::{Brand, Session};

// Re-export CommandMode from mayara-core
pub(crate) use mayara_core::protocol::furuno::command::CommandMode;

mod command;
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

const FURUNO_ANNOUNCE_MAYARA_PACKET: [u8; 32] = [
    0x1, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0, 0x18, 0x1, 0x0, 0x0, 0x0, b'M', b'A',
    b'Y', b'A', b'R', b'A', 0x0, 0x0, 0x1, 0x1, 0x0, 0x2, 0x0, 0x1, 0x0, 0x12,
];
const FURUNO_REQUEST_BEACON_PACKET: [u8; 16] = [
    0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x08, 0x01, 0x00, 0x00, 0x00,
];
const FURUNO_REQUEST_MODEL_PACKET: [u8; 16] = [
    0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x14, 0x01, 0x00, 0x08, 0x01, 0x00, 0x00, 0x00,
];

// CommandMode is now imported from mayara_core::protocol::furuno::command

#[allow(dead_code)]
enum RadarModel {
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
    fn to_str(&self) -> &str {
        match self {
            RadarModel::Unknown => "Unknown",
            RadarModel::FAR21x7 => "FAR21x7",
            RadarModel::DRS => "DRS",
            RadarModel::FAR14x7 => "FAR14x7",
            RadarModel::DRS4DL => "DRS4DL",
            RadarModel::FAR3000 => "FAR3000",
            RadarModel::DRS4DNXT => "DRS4DNXT",
            RadarModel::DRS6ANXT => "DRS6ANXT",
            RadarModel::DRS6AXCLASS => "DRS6AXCLASS",
            RadarModel::FAR15x3 => "FAR15x3",
            RadarModel::FAR14x6 => "FAR14x6",
            RadarModel::DRS12ANXT => "DRS12ANXT",
            RadarModel::DRS25ANXT => "DRS25ANXT",
        }
    }
}

// DRS-4D NXT
// [01, 00, 00, 01, 00, 00, 00, 00, 00, 01, 00, 18, 01, 00, 00, 00, 52, 44, 30, 30, 33, 32, 31, 32, 01, 01, 00, 02, 00, 01, 00, 12] len 32
// [ .   .   .   .   .   .   .   .   .   .   .   .   .   .   .   .   R   D   0   0   3   2   1   2   .   .   .   .   .   .   .   .]
//                                               ^__length           ^_name, always 8 long?
// FAR 2127
// [01, 00, 00, 01, 00, 00, 00, 00, 00, 01, 00, 1A, 01, 00, 00, 00, 52, 41, 44, 41, 52, 00, 00, 00, 01, 00, 00, 03, 00, 01, 00, 04, 00, 05] len 34
// [ .   .   .   .   .   .   .   .   .   .   .   .   .   .   .   .   R   A   D   A   R   .   .   .   .   .   .   .   .   .   .   .   .   .]
//
// TimeZero
// [01, 00, 00, 01, 00, 00, 00, 00, 00, 01, 00, 1C, 01, 00, 00, 00, 4D, 46, 30, 30, 33, 31, 35, 30, 01, 01, 00, 04, 00, 0B, 00, 15, 00, 14, 00, 16] len 36
// [ .   .   .   .   .   .   .   .   .   .   .   .   .   .   .   .   M   F   0   0   3   1   5   0   .   .   .   .   .   .   .   .   .   .   .   .]
#[derive(Deserialize, Debug, Copy, Clone)]
#[repr(packed)]
struct FurunoRadarReport {
    _header: [u8; 11],
    length: u8,
    _filler2: [u8; 4],
    name: [u8; 8],
    // ... followed by unknown stuff, looks like two bytes a piece. No idea what they are...
}

const FURUNO_RADAR_REPORT_HEADER: [u8; 11] =
    [0x1, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x0];
const FURUNO_RADAR_REPORT_LENGTH_MIN: usize = std::mem::size_of::<FurunoRadarReport>();

#[derive(Deserialize, Debug, Copy, Clone)]
#[repr(packed)]
// Length 170 bytes
struct FurunoRadarModelReport {
    _filler1: [u8; 24],
    model: [u8; 32],
    _firmware_versions: [u8; 32],
    _firmware_version: [u8; 32],
    serial_no: [u8; 32],
    _filler2: [u8; 18],
}

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
            .set_string(&crate::settings::ControlType::UserName, info.key())
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

        if report.len() >= FURUNO_RADAR_REPORT_LENGTH_MIN
            && report[16] == b'R'
            && report[0..11] == FURUNO_RADAR_REPORT_HEADER
        {
            self.process_beacon_report(report, from, via, radars, subsys)
        } else if report.len() == 170 {
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
        match deserialize::<FurunoRadarReport>(report) {
            Ok(data) => {
                if data.length as usize + 8 != report.len() {
                    log::error!(
                        "{}: Furuno report length mismatch: {} != {}",
                        from,
                        data.length,
                        report.len() - 8
                    );
                    return Ok(());
                }
                if let Some(name) = c_string(&data.name) {
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
                        Some(name),
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
                }
            }
            Err(e) => {
                log::error!(
                    "{} via {}: Failed to decode Furuno radar report: {}",
                    from,
                    nic_addr,
                    e
                );
            }
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
            match deserialize::<FurunoRadarModelReport>(report) {
                Ok(data) => {
                    let model = c_string(&data.model);
                    let serial_no = c_string(&data.serial_no);
                    log::debug!(
                        "{}: Furuno model report: {}",
                        from,
                        PrintableSlice::new(report)
                    );
                    log::debug!("{}: model: {:?}", from, model);
                    log::debug!("{}: serial_no: {:?}", from, serial_no);

                    if let Some(serial_no) = serial_no {
                        radars.update_serial_no(key, serial_no.to_string());
                    }

                    if let Some(_model) = model {
                        self.model_found = true;
                    }
                }
                Err(e) => {
                    log::error!(
                        "{} via {}: Failed to decode Furuno radar report: {}",
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
                    &FURUNO_REQUEST_BEACON_PACKET,
                    &FURUNO_REQUEST_MODEL_PACKET,
                    &FURUNO_ANNOUNCE_MAYARA_PACKET,
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
