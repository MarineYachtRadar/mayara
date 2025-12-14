use atomic_float::AtomicF64;
use futures_util::future::select_ok;
use mdns_sd::{Error, IfKind, ServiceDaemon, ServiceEvent};
use nmea_parser::*;
use serde_json::Value;
use std::{
    collections::HashSet,
    future::Future,
    io::ErrorKind,
    net::SocketAddr,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{io::AsyncBufReadExt, net::UdpSocket, time::sleep};
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio::{io::BufReader, sync::mpsc::Receiver};
use tokio_graceful_shutdown::SubsystemHandle;

use crate::{
    radar::{GeoPosition, RadarError},
    Session,
};

static HEADING_TRUE: AtomicF64 = AtomicF64::new(f64::NAN);
static POSITION_VALID: AtomicBool = AtomicBool::new(false);
static POSITION_LAT: AtomicF64 = AtomicF64::new(f64::NAN);
static POSITION_LON: AtomicF64 = AtomicF64::new(f64::NAN);
static COG: AtomicF64 = AtomicF64::new(f64::NAN);
static SOG: AtomicF64 = AtomicF64::new(f64::NAN);

pub(crate) fn get_heading_true() -> Option<f64> {
    let heading = HEADING_TRUE.load(Ordering::Acquire);
    if !heading.is_nan() {
        return Some(heading);
    }
    return None;
}

pub(crate) fn set_heading_true(heading: Option<f64>) {
    if let Some(h) = heading {
        HEADING_TRUE.store(h, Ordering::Release);
    } else {
        HEADING_TRUE.store(f64::NAN, Ordering::Release);
    }
}

pub(crate) fn get_radar_position() -> Option<GeoPosition> {
    if POSITION_VALID.load(Ordering::Acquire) {
        let lat = POSITION_LAT.load(Ordering::Acquire);
        let lon = POSITION_LON.load(Ordering::Acquire);
        return Some(GeoPosition::new(lat, lon));
    }
    return None;
}

pub(crate) fn get_position_i64() -> (Option<i64>, Option<i64>) {
    if POSITION_VALID.load(Ordering::Acquire) {
        let lat = POSITION_LAT.load(Ordering::Acquire);
        let lon = POSITION_LON.load(Ordering::Acquire);
        let lat = (lat * 1e16) as i64;
        let lon = (lon * 1e16) as i64;
        return (Some(lat), Some(lon));
    }
    return (None, None);
}

pub(crate) fn set_position(lat: Option<f64>, lon: Option<f64>) {
    if let (Some(lat), Some(lon)) = (lat, lon) {
        POSITION_LAT.store(lat, Ordering::Release);
        POSITION_LON.store(lon, Ordering::Release);
        POSITION_VALID.store(true, Ordering::Release);
    } else {
        POSITION_VALID.store(false, Ordering::Release);
        return;
    }
}

pub(crate) fn get_cog() -> Option<f64> {
    let cog = COG.load(Ordering::Acquire);
    if !cog.is_nan() {
        return Some(cog);
    }
    return None;
}

pub(crate) fn set_cog(cog: Option<f64>) {
    if let Some(c) = cog {
        COG.store(c, Ordering::Release);
    } else {
        COG.store(f64::NAN, Ordering::Release);
    }
}

pub(crate) fn get_sog() -> Option<f64> {
    let sog = SOG.load(Ordering::Acquire);
    if !sog.is_nan() {
        return Some(sog);
    }
    return None;
}

pub(crate) fn set_sog(sog: Option<f64>) {
    if let Some(s) = sog {
        SOG.store(s, Ordering::Release);
    } else {
        SOG.store(f64::NAN, Ordering::Release);
    }
}

/// The hostname of the devices we are searching for.
const SIGNAL_K_SERVICE_NAME: &'static str = "_signalk-tcp._tcp.local.";
const NMEA0183_SERVICE_NAME: &'static str = "_nmea-0183._tcp.local.";

const SUBSCRIBE: &'static str = "{\"context\": \"vessels.self\",
         \"subscribe\": [{\"path\": \"navigation.headingTrue\"},
                         {\"path\": \"navigation.position\"},
                         {\"path\": \"navigation.speedOverGround\"},
                         {\"path\": \"navigation.courseOverGroundTrue\"}]}\r\n";

enum ConnectionType {
    Mdns,
    Udp(SocketAddr),
    Tcp(SocketAddr),
}

impl ConnectionType {
    fn parse(interface: &Option<String>) -> ConnectionType {
        match interface {
            None => {
                return ConnectionType::Mdns;
            }
            Some(interface) => {
                let parts: Vec<&str> = interface.splitn(2, ':').collect();
                if parts.len() == 1 {
                    return ConnectionType::Mdns;
                } else if parts.len() == 2 {
                    if let Ok(addr) = parts[1].parse() {
                        // Dump if illegal address
                        match parts[0].to_ascii_lowercase().as_str() {
                            "udp" => return ConnectionType::Udp(addr),
                            "tcp" => return ConnectionType::Tcp(addr),
                            _ => {} // fallthrough to panic below
                        }
                    }
                }
            }
        }
        panic!("Interface must be either interface name (no :) or <connection>:<address>:<port> with <connection> one of `udp_listen`, `udp` or `tcp`.");
    }
}

#[derive(Debug)]
enum Stream {
    Tcp(TcpStream),
    Udp(UdpSocket),
}

pub(crate) struct NavigationData {
    session: Session,
    nmea0183_mode: bool,
    service_name: &'static str,
    what: &'static str,
    nmea_parser: Option<NmeaParser>,
}

impl NavigationData {
    pub(crate) fn new(session: Session) -> Self {
        let nmea0183 = session.read().unwrap().args.nmea0183;
        match nmea0183 {
            true => NavigationData {
                session,
                nmea0183_mode: true,
                service_name: NMEA0183_SERVICE_NAME,
                what: "NMEA0183",
                nmea_parser: Some(NmeaParser::new()),
            },
            false => NavigationData {
                session,
                nmea0183_mode: false,
                service_name: SIGNAL_K_SERVICE_NAME,
                what: "Signal K",
                nmea_parser: None,
            },
        }
    }

    pub(crate) async fn run(
        &mut self,
        subsys: SubsystemHandle,
        rx_ip_change: Receiver<()>,
    ) -> Result<(), Error> {
        log::debug!("{} run_loop (re)start", self.what);
        let mut rx_ip_change = rx_ip_change;
        let navigation_address = self.session.read().unwrap().args.navigation_address.clone();

        loop {
            match self
                .find_service(&subsys, &mut rx_ip_change, &navigation_address)
                .await
            {
                Ok(Stream::Tcp(stream)) => {
                    log::info!(
                        "Listening to {} data from {}",
                        self.what,
                        stream.peer_addr().unwrap()
                    );
                    match self.receive_loop(stream, &subsys).await {
                        Err(RadarError::Shutdown) => {
                            log::debug!("{} receive_loop shutdown", self.what);
                            return Ok(());
                        }
                        e => {
                            log::debug!("{} receive_loop restart on result {:?}", self.what, e);
                        }
                    }
                }
                Ok(Stream::Udp(socket)) => {
                    log::info!("Listening to {} data via UDP", self.what);
                    match self.receive_udp_loop(socket, &subsys).await {
                        Err(RadarError::Shutdown) => {
                            log::debug!("{} receive_loop shutdown", self.what);
                            return Ok(());
                        }
                        e => {
                            log::debug!("{} receive_loop restart on result {:?}", self.what, e);
                        }
                    }
                }
                Err(e) => match e {
                    RadarError::Shutdown => {
                        log::debug!("{} run_loop shutdown", self.what);
                        return Ok(());
                    }
                    e => {
                        log::debug!("{} find_service restart on result {:?}", self.what, e);
                    }
                },
            }
        }
    }

    async fn find_service(
        &self,
        subsys: &SubsystemHandle,
        rx_ip_change: &mut Receiver<()>,
        interface: &Option<String>,
    ) -> Result<Stream, RadarError> {
        let connection_type = ConnectionType::parse(interface);
        match connection_type {
            ConnectionType::Mdns => {
                self.find_mdns_service(subsys, rx_ip_change, interface)
                    .await
            }
            ConnectionType::Tcp(addr) => self.find_tcp_service(subsys, addr).await,
            ConnectionType::Udp(addr) => self.find_udp_service(subsys, addr).await,
        }
    }

    async fn find_mdns_service(
        &self,
        subsys: &SubsystemHandle,
        rx_ip_change: &mut Receiver<()>,
        interface: &Option<String>,
    ) -> Result<Stream, RadarError> {
        let mut known_addresses: HashSet<SocketAddr> = HashSet::new();

        let mdns = ServiceDaemon::new().expect("Failed to create daemon");

        if interface.is_some() {
            let _ = mdns.disable_interface(IfKind::All);
            let navigation_address = self
                .session
                .read()
                .unwrap()
                .args
                .navigation_address
                .as_ref()
                .unwrap()
                .to_string()
                .clone();
            let _ = mdns.enable_interface(IfKind::Name(navigation_address));
        }
        let tcp_locator = mdns.browse(self.service_name).expect(&format!(
            "Failed to browse for {} service",
            self.service_name
        ));

        log::debug!("SignalK find_service (re)start");

        let r: Result<Stream, RadarError>;
        loop {
            let s = &subsys;
            tokio::select! { biased;
                _ = s.on_shutdown_requested() => {
                    r = Err(RadarError::Shutdown);
                    break;
                },
                _ = rx_ip_change.recv() => {
                    log::debug!("rx_ip_change");
                    r = Err(RadarError::IPAddressChanged);
                    break;
                },
                event = tcp_locator.recv_async() => {
                    match event {
                        Ok(ServiceEvent::ServiceResolved(info)) => {
                            log::debug!("Resolved a new {} service: {}", self.what, info.get_fullname());
                            let addr = info.get_addresses();
                            let port = info.get_port();

                            for a in addr {
                                known_addresses.insert(SocketAddr::new(*a, port));
                            }
                        },
                        _ => {
                            continue;
                        }
                    }

                }
            }

            let stream = connect_first(known_addresses.clone()).await;
            match stream {
                Ok(stream) => {
                    log::info!(
                        "Listening to {} data from {}",
                        self.what,
                        stream.peer_addr().unwrap()
                    );

                    r = Ok(Stream::Tcp(stream));
                    break;
                }
                Err(_e) => {} // Just loop
            }
        }

        log::debug!("find_service(...,'{}') = {:?}", self.service_name, r);
        if let Ok(r3) = mdns.shutdown() {
            if let Ok(r3) = r3.recv() {
                log::debug!("mdns_shutdown: {:?}", r3);
            }
        }
        return r;
    }

    async fn find_tcp_service(
        &self,
        subsys: &SubsystemHandle,
        addr: SocketAddr,
    ) -> Result<Stream, RadarError> {
        log::debug!("TCP find_service {} (re)start", self.what);

        loop {
            let s = &subsys;

            tokio::select! { biased;
                _ = s.on_shutdown_requested() => {
                    return Err(RadarError::Shutdown);
                },
                stream = connect_to_socket(addr) => {
                    match stream {
                        Ok(stream) => {
                            log::info!(
                                "Receiving {} data from {}",
                                self.what,
                                stream.peer_addr().unwrap()
                            );
                            return Ok(Stream::Tcp(stream));
                        }
                        Err(e) => {
                            log::trace!("Failed to connect {} to {addr}: {e}", self.what);
                            sleep(Duration::from_millis(1000)).await;
                        }
                    }
                }
            }
        }
    }

    async fn find_udp_service(
        &self,
        subsys: &SubsystemHandle,
        addr: SocketAddr,
    ) -> Result<Stream, RadarError> {
        log::debug!("UDP find_service (re)start");

        loop {
            let s = &subsys;

            tokio::select! { biased;
                _ = s.on_shutdown_requested() => {
                    return Err(RadarError::Shutdown);
                },
                stream = UdpSocket::bind(addr) => {
                    match stream {
                        Ok(stream) => {
                            log::info!(
                                "Receiving {} data from {}",
                                self.what,
                                stream.local_addr().unwrap()
                            );
                            return Ok(Stream::Udp(stream));
                        }
                        Err(e) => {
                            log::trace!("Failed to bind {} to {addr}: {e}", self.what);
                            sleep(Duration::from_millis(1000)).await;
                        }
                    }
                }
            }
        }
    }

    // Loop until we get an error, then just return the error
    // or Ok if we are to shutdown.
    async fn receive_loop(
        &mut self,
        mut stream: TcpStream,
        subsys: &SubsystemHandle,
    ) -> Result<(), RadarError> {
        let (read_half, mut write_half) = stream.split();
        let mut lines = BufReader::new(read_half).lines();

        loop {
            tokio::select! { biased;
                _ = subsys.on_shutdown_requested() => {
                    log::debug!("{} receive_loop shutdown", self.what);
                    return Ok(());
                },
                r = lines.next_line() => {
                    match r {
                        Ok(Some(line)) => {
                            log::trace!("{} <- {}", self.what, line);
                            if self.nmea0183_mode {
                                // We are in NMEA0183 mode, so we need to parse
                                // the data we get.
                                match self.parse_nmea0183(&line) {
                                    Err(e) => { log::warn!("{}", e)}
                                    Ok(_) => { }
                                }
                            } else {
                                // We are in SignalK mode, so we need to subscribe
                                // to the data we want.
                                if line.starts_with("{\"name\":") {
                                    self.send_subscription(&mut write_half).await?;
                                    log::trace!("{} -> {}", self.what, SUBSCRIBE);
                                }
                                else {
                                    match parse_signalk(&line) {
                                        Err(e) => { log::warn!("{}", e)}
                                        Ok(_) => { }
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            return Ok(());
                        }
                        Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                            continue;
                        }
                        Err(e) => {
                            return Err(e.into());
                        }
                    }
                }
            }
        }
    }

    async fn send_subscription(
        &self,
        stream: &mut tokio::net::tcp::WriteHalf<'_>,
    ) -> Result<(), RadarError> {
        let bytes: &[u8] = SUBSCRIBE.as_bytes();

        stream.write_all(bytes).await.map_err(|e| RadarError::Io(e))
    }

    // Loop until we get an error, then just return the error
    // or Ok if we are to shutdown.
    async fn receive_udp_loop(
        &mut self,
        socket: UdpSocket,
        subsys: &SubsystemHandle,
    ) -> Result<(), RadarError> {
        loop {
            tokio::select! { biased;
                _ = subsys.on_shutdown_requested() => {
                    log::debug!("{} receive_loop shutdown", self.what);
                    return Ok(());
                },
                r = socket.readable() => {
                    match r {
                        Ok(()) => {
                            let mut buf = [0; 2000];
                            let r = socket.try_recv(&mut buf);
                            match r {
                                Ok(len) => {
                                    self.process_udp_buf(&buf[..len]);
                                },
                                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {},
                                Err(e) => { log::warn!("{}", e)}
                            }
                        }

                        Err(e) => {
                            return Err(e.into());
                        }
                    }
                }
            }
        }
    }

    fn process_udp_buf(&mut self, buf: &[u8]) {
        if let Ok(data) = String::from_utf8(buf.to_vec()) {
            for line in data.lines() {
                match if self.nmea0183_mode {
                    self.parse_nmea0183(line)
                } else {
                    parse_signalk(&line)
                } {
                    Err(e) => {
                        log::warn!("{}", e)
                    }
                    Ok(_) => {}
                }
            }
        }
    }

    fn parse_nmea0183(&mut self, s: &str) -> Result<(), RadarError> {
        let parser = self.nmea_parser.as_mut().unwrap();

        match parser.parse_sentence(s) {
            Ok(ParsedMessage::Rmc(rmc)) => {
                set_position(rmc.latitude, rmc.longitude);
            }
            Ok(ParsedMessage::Gll(gll)) => {
                set_position(gll.latitude, gll.longitude);
            }
            Ok(ParsedMessage::Hdt(hdt)) => {
                set_heading_true(hdt.heading_true);
            }
            Ok(ParsedMessage::Vtg(vtg)) => {
                set_cog(vtg.cog_true);
                let sog = vtg
                    .sog_kph
                    .or_else(|| vtg.sog_knots.map(|k| k * 1.852))
                    .map(|s| s * 3.6); // convert to m/s
                set_sog(sog);
            }

            Err(e) => match e {
                ParseError::UnsupportedSentenceType(_) => {}
                ParseError::CorruptedSentence(e2) => {
                    return Err(RadarError::ParseNmea0183(format!("{s}: {e2}")));
                }
                ParseError::InvalidSentence(e2) => {
                    return Err(RadarError::ParseNmea0183(format!("{s}: {e2}")));
                }
            },
            _ => {}
        }
        Ok(())
    }
}

//  {"context":"vessels.urn:mrn:imo:mmsi:244060807","updates":
//   [{"source":{"sentence":"GLL","talker":"BM","type":"NMEA0183","label":"canboat-merrimac"},
//     "$source":"canboat-merrimac.BM","timestamp":"2024-10-01T09:11:36.000Z",
//     "values":[{"path":"navigation.position","value":{"longitude":5.428445,"latitude":53.180205}}]}]}

fn parse_signalk(s: &str) -> Result<(), RadarError> {
    match serde_json::from_str::<Value>(s) {
        Ok(v) => {
            let updates = &v["updates"][0];
            let values = &updates["values"][0];
            {
                log::trace!("values: {:?}", values);

                if let (Some(path), value) = (values["path"].as_str(), &values["value"]) {
                    match path {
                        "navigation.position" => {
                            set_position(value["longitude"].as_f64(), value["latitude"].as_f64());
                            return Ok(());
                        }
                        "navigation.headingTrue" => {
                            set_heading_true(value.as_f64());
                            return Ok(());
                        }
                        "navigation.speedOverGround" => {
                            set_sog(value.as_f64());
                            return Ok(());
                        }
                        "navigation.courseOverGroundTrue" => {
                            set_cog(value.as_f64());
                            return Ok(());
                        }
                        _ => {
                            return Err(RadarError::ParseJson(format!("Ignored path '{}'", path)));
                        }
                    }
                }
            }
        }
        Err(e) => {
            log::warn!("Unable to parse SK message '{}'", s);
            return Err(RadarError::ParseJson(e.to_string()));
        }
    }
    return Err(RadarError::ParseJson(format!(
        "Insufficient fields in '{}'",
        s
    )));
}

async fn connect_to_socket(address: SocketAddr) -> Result<TcpStream, RadarError> {
    let stream = TcpStream::connect(address)
        .await
        .map_err(|e| RadarError::Io(e))?;
    log::debug!("Connected to {}", address);
    Ok(stream)
}

///
/// Take an interable of SocketAddr and return a TCP stream to the first socket that connects.
///
async fn connect_first<I>(addresses: I) -> Result<TcpStream, RadarError>
where
    I: IntoIterator<Item = SocketAddr>,
{
    // Create a collection of connection futures
    // Since the life time of the stream must outlive this function,
    // and we create async closures on the stack, we must add a lot
    // of syntactic sugar so the compiler doesn't grumble.
    // Future<....> says that it is async, e.g. first call returns a future.
    // It resolves to Output = ... and is Send.
    // Box<> places this on the heap, not stack.
    // Pin<> makes sure it doesn't move or get invalid as an object.
    // Vec<> so we can store a list of these.
    let futures: Vec<Pin<Box<dyn Future<Output = Result<TcpStream, RadarError>> + Send>>> =
        addresses
            .into_iter()
            .map(|address| {
                log::debug!("Connecting to {}", address);
                Box::pin(connect_to_socket(address)) as Pin<Box<dyn Future<Output = _> + Send>>
            })
            .collect();

    // Use select_ok to return the first successful connection
    match select_ok(futures).await {
        Ok((stream, _)) => {
            log::debug!("First successful connection: {:?}", stream);
            Ok(stream)
        }
        Err(e) => {
            log::debug!("All connections failed: {}", e);
            Err(e)
        }
    }
}
