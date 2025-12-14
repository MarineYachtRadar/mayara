use std::mem::transmute;
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::net::UdpSocket;

use crate::brand::navico::{NAVICO_INFO_ADDRESS, NAVICO_SPEED_ADDRESS_A, NAVICO_SPEED_ADDRESS_B};
use crate::navdata::{get_cog, get_heading_true, get_sog};
use crate::network::create_multicast_send;
use crate::radar::{RadarError, RadarInfo};

#[derive(Debug)]
#[repr(packed)]
#[allow(dead_code)]
pub(crate) struct HaloHeadingPacket {
    marker: [u8; 4],   // 4 bytes containing 'NKOE'
    preamble: [u8; 4], // 4 bytes containing '00 01 90 02'
    counter: [u8; 2],  // 2 byte counter incrementing by 1 every transmission, in BigEndian
    u01: [u8; 26],     // 25 bytes of unknown stuff that doesn't seem to vary
    u02: [u8; 4],      // 4 bytes containing '12 f1 01 00'
    now: [u8; 8],      // 8 bytes containing millis since 1970
    u03: [u8; 8],      // 8 bytes containing 2
    u04: [u8; 4],      // 4 bytes containing some fixed data, could be position?
    u05: [u8; 4],      // 4 bytes containing some fixed data, could be position?
    u06: [u8; 1],      // 1 byte containing counter or 0xff
    heading: [u8; 2],  // 2 bytes containing heading
    u07: [u8; 5],      // 5 bytes containing varying unknown data
}

impl HaloHeadingPacket {
    pub fn transmute(bytes: &[u8]) -> Result<Self, anyhow::Error> {
        // This is safe as the struct's bits are always all valid representations,
        // or we convert them using a fail safe function
        Ok(unsafe {
            let report: [u8; 72] = bytes.try_into()?;
            transmute(report)
        })
    }
}

#[derive(Debug)]
#[repr(packed)]
#[allow(dead_code)]
pub(crate) struct HaloNavigationPacket {
    marker: [u8; 4],   // 4 bytes containing 'NKOE'
    preamble: [u8; 4], // 4 bytes containing '00 01 90 02'
    counter: [u8; 2],  // 2 byte counter incrementing by 1 every transmission, in BigEndian
    u01: [u8; 26],     // 26 bytes of unknown stuff that doesn't seem to vary
    u02: [u8; 4],      // 2 bytes containing '02 f8 01 00'
    now: [u8; 8],      // 8 bytes containing millis since 1970
    u03: [u8; 18],     // 18 bytes containing ?
    pub cog: [u8; 2],  // u16 containing COG in 0.01 radians, 0..63488
    pub sog: [u8; 2],  // u16 containing SOG in 0.01 m/s, 0..65535
    u04: [u8; 2],      // 2 bytes containg 0xff 0xff
}

impl HaloNavigationPacket {
    pub fn transmute(bytes: &[u8]) -> Result<Self, anyhow::Error> {
        // This is safe as the struct's bits are always all valid representations,
        // or we convert them using a fail safe function
        Ok(unsafe {
            let report: [u8; 72] = bytes.try_into()?;
            transmute(report)
        })
    }
}

#[derive(Debug)]
#[repr(packed)]
#[allow(dead_code)]
pub(crate) struct HaloSpeedPacket {
    marker: [u8; 6],  // 6 bytes containing '01 d3 01 00 00 00'
    pub sog: [u8; 2], // Speed m/s
    u00: [u8; 6],     // 6 bytes containing '00 00 01 00 00 00'
    pub cog: [u8; 2], // COG
    u01: [u8; 7],     // 6 bytes containing '00 00 01 33 00 00 00'
}

impl HaloSpeedPacket {
    pub fn transmute(bytes: &[u8]) -> Result<Self, anyhow::Error> {
        // This is safe as the struct's bits are always all valid representations,
        // or we convert them using a fail safe function
        Ok(unsafe {
            let report: [u8; 23] = bytes.try_into()?;
            transmute(report)
        })
    }
}

// This enum is an index into the SOCKET_ADDRESS array
enum SocketIndex {
    HeadingAndNavigation,
    SpeedA,
    SpeedB,
}

const SOCKET_ADDRESS: [SocketAddrV4; 3] = [
    NAVICO_INFO_ADDRESS,
    NAVICO_SPEED_ADDRESS_A,
    NAVICO_SPEED_ADDRESS_B,
];

pub(crate) struct Information {
    key: String,
    nic_addr: Ipv4Addr,
    sock: [Option<UdpSocket>; 3], // Heading/Navigation, Speed A, Speed B
    counter: u16,
}

unsafe fn any_as_u8_slice<T: Sized>(p: &T) -> &[u8] {
    ::core::slice::from_raw_parts((p as *const T) as *const u8, ::core::mem::size_of::<T>())
}

impl Information {
    pub fn new(key: String, info: &RadarInfo) -> Self {
        Information {
            key,
            nic_addr: info.nic_addr.clone(),
            sock: [None, None, None], // Heading/Navigation and Speed A/B
            counter: 0,
        }
    }

    async fn start_socket(&mut self, index: usize) -> Result<(), RadarError> {
        if self.sock[index].is_some() {
            return Ok(());
        }
        match create_multicast_send(&SOCKET_ADDRESS[index], &self.nic_addr) {
            Ok(sock) => {
                log::debug!(
                    "{} {} via {}: sending info",
                    self.key,
                    &SOCKET_ADDRESS[index],
                    &self.nic_addr
                );
                self.sock[index] = Some(sock);

                Ok(())
            }
            Err(e) => {
                log::debug!(
                    "{} {} via {}: create multicast failed: {}",
                    self.key,
                    &SOCKET_ADDRESS[index],
                    &self.nic_addr,
                    e
                );
                Err(RadarError::Io(e))
            }
        }
    }

    pub async fn send(&mut self, index: usize, message: &[u8]) -> Result<(), RadarError> {
        self.start_socket(index).await?;

        if let Some(sock) = &self.sock[index] {
            sock.send(message).await.map_err(RadarError::Io)?;
            log::trace!("{}: sent {:02X?}", self.key, message);
        }

        Ok(())
    }

    async fn send_heading_packet(&mut self) -> Result<(), RadarError> {
        if let Some(heading) = get_heading_true() {
            let heading = (heading * 10.0) as i16;
            let now = chrono::Utc::now().timestamp_millis().to_le_bytes();
            let heading_packet = HaloHeadingPacket {
                marker: [b'N', b'K', b'O', b'E'],
                preamble: [0, 1, 0x90, 0x02],
                counter: self.counter.to_be_bytes(),
                u01: [0; 26], // 25 bytes of unknown data
                u02: [0x12, 0xf1, 0x01, 0x00],
                now,
                u03: [0, 0, 0, 2, 0, 0, 0, 0], // 8 bytes of unknown data
                u04: [0; 4],                   // 4 bytes of unknown data
                u05: [0; 4],                   // 4 bytes of unknown data
                u06: [0xff],                   // 1 byte, could be a counter or 0xff
                heading: heading.to_le_bytes(), // 2 bytes for heading
                u07: [0; 5],                   // 5 bytes of unknown data
            };

            let bytes: &[u8] = unsafe { any_as_u8_slice(&heading_packet) };
            self.counter = self.counter.wrapping_add(1);

            self.send(SocketIndex::HeadingAndNavigation as usize, bytes)
                .await?;
        }
        Ok(())
    }

    async fn send_navigation_packet(&mut self) -> Result<(), RadarError> {
        if let (Some(sog), Some(cog)) = (get_sog(), get_cog()) {
            let sog = (sog * 10.0) as i16;
            let cog = (cog * (63488.0 / 360.0)) as i16;
            let now = chrono::Utc::now().timestamp_millis().to_le_bytes();
            let heading_packet = HaloNavigationPacket {
                marker: [b'N', b'K', b'O', b'E'],
                preamble: [0, 1, 0x90, 0x02],
                counter: self.counter.to_be_bytes(),
                u01: [0; 26], // 25 bytes of unknown data
                u02: [0x02, 0xf8, 0x01, 0x00],
                now,
                u03: [0; 18],           // 8 bytes of unknown data
                cog: cog.to_le_bytes(), // 2 bytes for COG
                sog: sog.to_le_bytes(), // 2 bytes for SOG
                u04: [0xff, 0xff],      // 5 bytes of unknown data
            };

            let bytes: &[u8] = unsafe { any_as_u8_slice(&heading_packet) };
            self.counter = self.counter.wrapping_add(1);

            self.send(SocketIndex::HeadingAndNavigation as usize, bytes)
                .await?;
        }
        Ok(())
    }

    async fn send_speed_packet(&mut self) -> Result<(), RadarError> {
        if let (Some(sog), Some(cog)) = (get_sog(), get_cog()) {
            let sog = (sog * 10.0) as u16;
            let cog = (cog * 63488.0 / 360.0) as u16;
            let speed_packet = HaloSpeedPacket {
                marker: [0x01, 0xd3, 0x01, 0x00, 0x00, 0x00],
                sog: sog.to_le_bytes(),
                u00: [0x00, 0x00, 0x01, 0x00, 0x00, 0x00], // 6 bytes of unknown data
                cog: cog.to_le_bytes(),
                u01: [0x00, 0x00, 0x01, 0x33, 0x00, 0x00, 0x00],
            };

            let bytes: &[u8] = unsafe { any_as_u8_slice(&speed_packet) };

            self.send(SocketIndex::SpeedA as usize, bytes).await?;
            self.send(SocketIndex::SpeedB as usize, bytes).await?;
        }
        Ok(())
    }

    pub(super) async fn send_info_requests(&mut self) -> Result<(), RadarError> {
        self.send_heading_packet().await?;
        self.send_navigation_packet().await?;
        self.send_speed_packet().await?;
        Ok(())
    }
}
