use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::{fmt, io};
use tokio_graceful_shutdown::{SubsystemBuilder, SubsystemHandle};

use crate::locator::{LocatorAddress, LocatorId, RadarLocator, RadarLocatorState};
use crate::radar::{RadarInfo, SharedRadars};
use crate::util::PrintableSlice;
use crate::{Brand, Session};

mod data;
mod info;
mod report;
mod settings;

const NAVICO_SPOKES: usize = 2048;

// Length of a spoke in pixels. Every pixel is 4 bits (one nibble.)
const NAVICO_SPOKE_LEN: usize = 1024;

// Spoke numbers go from [0..4096>, but only half of them are used.
// The actual image is 2048 x 1024 x 4 bits
const NAVICO_BITS_PER_PIXEL: usize = BITS_PER_NIBBLE;

const SPOKES_PER_FRAME: usize = 32;
const BITS_PER_BYTE: usize = 8;
const BITS_PER_NIBBLE: usize = 4;
const NAVICO_PIXELS_PER_BYTE: usize = BITS_PER_BYTE / NAVICO_BITS_PER_PIXEL;
const RADAR_LINE_DATA_LENGTH: usize = NAVICO_SPOKE_LEN / NAVICO_PIXELS_PER_BYTE;

const NAVICO_BEACON_ADDRESS: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(236, 6, 7, 5)), 6878);

/* NAVICO API SPOKES */
/*
 * Data coming from radar is always 4 bits, packed two per byte.
 * The values 14 and 15 may be special depending on DopplerMode (only on HALO).
 *
 * To support targets, target trails and doppler we map those values 0..15 to
 * a
 */

/*
RADAR REPORTS

The radars send various reports. The first 2 bytes indicate what the report type is.
The types seen on a BR24 are:

2nd byte C4:   01 02 03 04 05 07 08
2nd byte F5:   08 0C 0D 0F 10 11 12 13 14

Not definitive list for
4G radars only send the C4 data.
*/

const NAVICO_ADDRESS_REQUEST_PACKET: [u8; 2] = [0x01, 0xB1];

// BR24 beacon comes from a different multicast address
const NAVICO_BR24_BEACON_ADDRESS: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(236, 6, 7, 4)), 6768);

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Model {
    Unknown,
    BR24,
    Gen3,
    Gen4,
    HALO,
}

const BR24_MODEL_NAME: &str = "BR24";

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Model::Unknown => "",
            Model::BR24 => BR24_MODEL_NAME,
            Model::Gen3 => "3G",
            Model::Gen4 => "4G",
            Model::HALO => "HALO",
        };
        write!(f, "{}", s)
    }
}

impl Model {
    pub fn new(s: &str) -> Self {
        match s {
            BR24_MODEL_NAME => Model::BR24,
            "3G" => Model::Gen3,
            "4G" => Model::Gen4,
            "HALO" => Model::HALO,
            _ => Model::Unknown,
        }
    }
}

#[derive(Clone)]
struct NavicoLocatorState {
    session: Session,
}

impl NavicoLocatorState {
    fn process_locator_report(
        &self,
        report: &[u8],
        from: &SocketAddrV4,
        via: &Ipv4Addr,
        radars: &SharedRadars,
        subsys: &SubsystemHandle,
    ) -> io::Result<()> {
        if report.len() < 2 {
            return Ok(());
        }

        log::trace!(
            "{}: Navico report: {:02X?} len {}",
            from,
            report,
            report.len()
        );
        log::trace!("{}: printable:     {}", from, PrintableSlice::new(report));

        if report == NAVICO_ADDRESS_REQUEST_PACKET {
            log::trace!("Radar address request packet from {}", from);
            return Ok(());
        }
        if report[0] == 0x1 && report[1] == 0xB2 {
            // Common Navico message

            return self.process_beacon_report(report, from, via, radars, subsys);
        }
        Ok(())
    }

    fn process_beacon_report(
        &self,
        report: &[u8],
        from: &SocketAddrV4,
        via: &Ipv4Addr,
        radars: &SharedRadars,
        subsys: &SubsystemHandle,
    ) -> Result<(), io::Error> {
        // Use core parsing for beacon
        use mayara_core::protocol::navico::parse_beacon_endpoints;

        let beacon = match parse_beacon_endpoints(report) {
            Ok(b) => b,
            Err(e) => {
                log::debug!(
                    "{} via {}: Failed to parse beacon: {}",
                    from,
                    via,
                    e
                );
                return Ok(());
            }
        };

        log::debug!("{} via {}: Beacon parsed: {:?}", from, via, beacon);

        let locator_id = if beacon.is_br24 {
            LocatorId::GenBR24
        } else {
            LocatorId::Gen3Plus
        };

        let model_name = if beacon.is_br24 {
            Some(BR24_MODEL_NAME)
        } else {
            None
        };

        // Parse radar address (must be IPv4)
        let radar_addr: SocketAddrV4 = beacon.radar_addr.parse().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Invalid radar addr: {}", e))
        })?;

        for radar_endpoint in beacon.radars {
            let data_addr: SocketAddrV4 = radar_endpoint.data_addr.parse().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Invalid data addr: {}", e))
            })?;
            let report_addr: SocketAddrV4 = radar_endpoint.report_addr.parse().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Invalid report addr: {}", e))
            })?;
            let send_addr: SocketAddrV4 = radar_endpoint.send_addr.parse().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Invalid send addr: {}", e))
            })?;

            let location_info: RadarInfo = RadarInfo::new(
                self.session.clone(),
                locator_id,
                Brand::Navico,
                Some(&beacon.serial_no),
                radar_endpoint.suffix.as_deref(),
                16,
                NAVICO_SPOKES,
                NAVICO_SPOKE_LEN,
                radar_addr,
                via.clone(),
                data_addr,
                report_addr,
                send_addr,
                settings::new(self.session.clone(), model_name),
                beacon.is_dual_range,
            );
            self.found(location_info, radars, subsys);
        }

        Ok(())
    }

    fn found(&self, info: RadarInfo, radars: &SharedRadars, subsys: &SubsystemHandle) {
        info.controls
            .set_string("userName", info.key())
            .unwrap();

        if let Some(mut info) = radars.located(info) {
            // It's new, start the RadarProcessor thread

            // Load the model name afresh, it may have been modified from persisted data
            let model = match info.controls.model_name() {
                Some(s) => Model::new(&s),
                None => Model::Unknown,
            };
            if model != Model::Unknown {
                let info2 = info.clone();
                settings::update_when_model_known(&mut info.controls, model, &info2);
                info.set_doppler(model == Model::HALO);
                radars.update(&info);
            }

            let data_name = info.key() + " data";
            let report_name = info.key() + " reports";
            let info_clone = info.clone();

            if self.session.read().unwrap().args.output {
                let info_clone2 = info.clone();

                subsys.start(SubsystemBuilder::new("stdout", move |s| {
                    info_clone2.forward_output(s)
                }));
            }

            let data_receiver = data::NavicoDataReceiver::new(&self.session, info);
            let report_receiver = report::NavicoReportReceiver::new(
                self.session.clone(),
                info_clone,
                radars.clone(),
                model,
            );

            subsys.start(SubsystemBuilder::new(
                data_name,
                move |s: SubsystemHandle| data_receiver.run(s),
            ));
            subsys.start(SubsystemBuilder::new(report_name, |s| {
                report_receiver.run(s)
            }));
        }
    }
}

impl RadarLocatorState for NavicoLocatorState {
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
        Box::new(NavicoLocatorState {
            session: self.session.clone(),
        }) // Navico is stateless
    }
}

#[derive(Clone)]
struct NavicoLocator {
    session: Session,
}

impl RadarLocator for NavicoLocator {
    fn set_listen_addresses(&self, addresses: &mut Vec<LocatorAddress>) {
        let mut beacon_request_packets: Vec<&'static [u8]> = Vec::new();
        if !self.session.read().unwrap().args.replay {
            beacon_request_packets.push(&NAVICO_ADDRESS_REQUEST_PACKET);
        };
        if !addresses.iter().any(|i| i.id == LocatorId::Gen3Plus) {
            addresses.push(LocatorAddress::new(
                LocatorId::Gen3Plus,
                &NAVICO_BEACON_ADDRESS,
                Brand::Navico,
                beacon_request_packets,
                Box::new(NavicoLocatorState {
                    session: self.session.clone(),
                }),
            ));
        }
    }
}

pub fn create_locator(session: Session) -> Box<dyn RadarLocator + Send> {
    let locator = NavicoLocator { session };
    Box::new(locator)
}

#[derive(Clone)]
struct NavicoBR24Locator {
    session: Session,
}

impl RadarLocator for NavicoBR24Locator {
    fn set_listen_addresses(&self, addresses: &mut Vec<LocatorAddress>) {
        if !addresses.iter().any(|i| i.id == LocatorId::GenBR24) {
            addresses.push(LocatorAddress::new(
                LocatorId::GenBR24,
                &NAVICO_BR24_BEACON_ADDRESS,
                Brand::Navico,
                vec![&NAVICO_ADDRESS_REQUEST_PACKET],
                Box::new(NavicoLocatorState {
                    session: self.session.clone(),
                }),
            ));
        }
    }
}

pub fn create_br24_locator(session: Session) -> Box<dyn RadarLocator + Send> {
    let locator = NavicoBR24Locator { session };
    Box::new(locator)
}

const BLANKING_SETS: [(usize, &str, &str); 4] = [
    (0, "noTransmitStart1", "noTransmitEnd1"),
    (1, "noTransmitStart2", "noTransmitEnd2"),
    (2, "noTransmitStart3", "noTransmitEnd3"),
    (3, "noTransmitStart4", "noTransmitEnd4"),
];
