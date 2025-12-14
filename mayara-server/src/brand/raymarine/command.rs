use tokio::net::UdpSocket;

use crate::network::create_multicast_send;
use crate::radar::range::Ranges;
use crate::radar::{RadarError, RadarInfo};
use crate::settings::{ControlValue, SharedControls};

use super::BaseModel;

mod quantum;
mod rd;

pub struct Command {
    key: String,
    info: RadarInfo,
    model: BaseModel,
    sock: Option<UdpSocket>,
}

impl Command {
    pub fn new(info: RadarInfo, model: BaseModel) -> Self {
        Command {
            key: info.key(),
            info,
            model,
            sock: None,
        }
    }

    #[allow(dead_code)]
    pub fn set_ranges(&mut self, ranges: Ranges) {
        self.info.ranges = ranges;
    }

    async fn start_socket(&mut self) -> Result<(), RadarError> {
        match create_multicast_send(&self.info.send_command_addr, &self.info.nic_addr) {
            Ok(sock) => {
                log::debug!(
                    "{} {} via {}: sending commands",
                    self.key,
                    &self.info.send_command_addr,
                    &self.info.nic_addr
                );
                self.sock = Some(sock);

                Ok(())
            }
            Err(e) => {
                log::debug!(
                    "{} {} via {}: create multicast failed: {}",
                    self.key,
                    &self.info.send_command_addr,
                    &self.info.nic_addr,
                    e
                );
                Err(RadarError::Io(e))
            }
        }
    }

    pub async fn send(&mut self, message: &[u8]) -> Result<(), RadarError> {
        if self.sock.is_none() {
            self.start_socket().await?;
        }
        if let Some(sock) = &self.sock {
            sock.send(message).await.map_err(RadarError::Io)?;
            log::trace!("{}: sent {:02X?}", self.key, message);
        }

        Ok(())
    }

    fn scale_100_to_byte(a: f32) -> u8 {
        // Map range 0..100 to 0..255
        let mut r = a * 255.0 / 100.0;
        if r > 255.0 {
            r = 255.0;
        } else if r < 0.0 {
            r = 0.0;
        }
        r as u8
    }

    #[allow(dead_code)]
    async fn send_no_transmit_cmd(
        &mut self,
        value_start: i16,
        value_end: i16,
        enabled: u8,
        sector: u8,
    ) -> Result<Vec<u8>, RadarError> {
        let mut cmd = Vec::with_capacity(12);

        cmd.extend_from_slice(&[0x0d, 0xc1, sector, 0, 0, 0, enabled]);
        self.send(&cmd).await?;
        cmd.clear();
        cmd.extend_from_slice(&[0xc0, 0xc1, sector, 0, 0, 0, enabled]);
        cmd.extend_from_slice(&value_start.to_le_bytes());
        cmd.extend_from_slice(&value_end.to_le_bytes());

        Ok(cmd)
    }

    pub async fn set_control(
        &mut self,
        cv: &ControlValue,
        controls: &SharedControls,
    ) -> Result<(), RadarError> {
        let value = cv
            .value
            .parse::<f32>()
            .map_err(|_| RadarError::MissingValue(cv.id.clone()))?;

        match self.model {
            BaseModel::RD => rd::set_control(self, cv, value, controls).await,
            BaseModel::Quantum => quantum::set_control(self, cv, value, controls).await,
        }
    }

    #[allow(dead_code)]
    pub(super) async fn send_report_requests(&mut self) -> Result<(), RadarError> {
        Ok(())
    }
}
