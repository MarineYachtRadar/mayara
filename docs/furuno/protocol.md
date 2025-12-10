# Furuno Radar Protocol Documentation

This document describes the Furuno radar network protocol as reverse-engineered from
network captures and the mayara-lib implementation.

## Network Architecture

Furuno radars operate on a dedicated network subnet, typically `172.31.0.0/16`.

### Ports

| Port | Protocol | Direction | Purpose |
|------|----------|-----------|---------|
| 10000 | TCP | Radar → | Login port (older models) |
| 10010 | TCP/UDP | Radar → | Login port (DRS-NXT) / Beacon (UDP) |
| 10001+ | TCP | Radar → | Command/report port (dynamic) |
| 10021 | UDP | Broadcast | NMEA position data (GGA, VTG) |
| 10024 | UDP | Multicast | Radar spoke data (239.255.0.2) |
| 10034 | UDP | Broadcast | ARPA target data |
| 10036 | UDP | Broadcast | NMEA heading data (HDT) |
| 33000 | UDP | Broadcast | TimeZero Sync (chart plotter) |

## Device Discovery (UDP Port 10010)

### Beacon Request Packet (16 bytes)
Sent by clients to request radar beacons:
```
01 00 00 01 00 00 00 00 01 01 00 08 01 00 00 00
```

### Model Request Packet (16 bytes)
Sent to request detailed model information:
```
01 00 00 01 00 00 00 00 14 01 00 08 01 00 00 00
```

### Announce Packet (32 bytes)
Sent by clients to announce presence (required before TCP connection):
```
01 00 00 01 00 00 00 00 00 01 00 18 01 00 00 00
4D 41 59 41 52 41 00 00 01 01 00 02 00 01 00 12
       M  A  Y  A  R  A
```

### Beacon Response (32 bytes)
Sent by radar to announce presence:
```
01 00 00 01 00 00 00 00 00 01 00 18 01 00 00 00
52 44 30 30 33 32 31 32 01 01 00 02 00 01 00 12
 R  D  0  0  3  2  1  2
```

### Model Response (170 bytes)
Contains detailed information including:
- Offset 0x26-0x2B: MAC address (6 bytes)
- Offset 0x30: Device name (32 bytes, null-terminated)
- Firmware versions, serial number

Example from DRS4D-NXT:
```
MAC: 00:d0:1d:4a:be:bb (Furuno OUI)
Name: DRS4D-NXT
Firmware: 01.01:01.01:01.05:01.05
Serial: 6424
```

## TCP Login Protocol

### Login Port Selection

**Important**: Different Furuno radar models use different TCP login ports:

| Model | Login Port | Notes |
|-------|------------|-------|
| DRS4D-NXT | 10010 | Same as beacon port |
| DRS6A-NXT | 10010 | Same as beacon port |
| Older models | 10000 | Base port |

The implementation should try port 10010 first, then fall back to 10000.

### Login Sequence

1. Client connects to TCP login port (10010 or 10000)
2. Client sends 56-byte LOGIN_MESSAGE containing copyright string
3. Radar responds with 12-byte response containing dynamic command port
4. Client disconnects and reconnects to command port

### Login Message (56 bytes)
```
08 01 00 38 01 00 00 00 00 01 00 00
COPYRIGHT (C) 2001 FURUNO ELECTRIC CO.,LTD.
```

### Login Response (12 bytes)
```
09 01 00 0c 01 00 00 00 XX XX YY YY
                        ^port offset
```
Command port = BASE_PORT (10000) + port_offset

## Command Protocol (TCP)

Commands are ASCII strings terminated with `\r\n`.

### Command Format
```
${mode}{command_id},{arg1},{arg2},...\r\n
```

### Command Modes
| Mode | Char | Description |
|------|------|-------------|
| Set | S | Set a value |
| Request | R | Request current value |
| New | N | Response/notification |

### Command IDs (hex)
| ID | Name | Description |
|----|------|-------------|
| 60 | Connect | Connection handshake |
| 61 | DispMode | Display mode |
| 62 | Range | Range setting |
| 63 | Gain | Gain control |
| 64 | Sea | Sea clutter |
| 65 | Rain | Rain clutter |
| 66 | CustomPictureAll | All picture settings |
| 67 | NoiseReduction | Noise Reduction ON/OFF |
| 69 | Status | Transmit/standby status |
| 77 | BlindSector | No-transmit zones |
| 83 | MainBangSize | Main bang suppression |
| 84 | AntennaHeight | Antenna height setting |
| 85 | NearSTC | Near STC |
| 86 | MiddleSTC | Middle STC |
| 87 | FarSTC | Far STC |
| 89 | AntennaRevolution | Scan speed |
| 96 | Modules | Module information |
| E3 | AliveCheck | Keep-alive (every 5s) |
| EE | RezBoost | RezBoost level (0=OFF, 1=Low, 2=Med, 3=High) |

### Common Commands

#### Transmit/Standby (0x69)
```
$S69,{status},0,0,60,300,0\r\n
```
- status=1: Standby
- status=2: Transmit

Response:
```
$N69,{status},0,0,60,300,0\r\n
```

#### Range (0x62)
```
$S62,{range_index},0,0\r\n
```
Response:
```
$N62,{range_index},0,0\r\n
```

#### Gain (0x63)
```
$S63,0,{value},{auto},{value},0\r\n
```
- value: 0-100
- auto: 0=manual, 1=auto

#### Keep-Alive (0xE3)
```
$RE3\r\n
```
Must be sent every 5 seconds.

## Complete Command Reference

Based on network captures from TimeZero Professional controlling a DRS4D-NXT radar.

### Command Port

The command port is **dynamic**. After login to port 10000, the radar returns a port
offset. In captures, port **10100** was observed (offset = 100).

### All Command IDs

| ID (hex) | Name | Format | Description |
|----------|------|--------|-------------|
| 00 | Named | `$R00,{name},` / `$N00,{name},{value}` | Named parameters (Fan1Status, etc.) |
| 61 | DispMode | `$R61,0,0,{idx}` | Display mode |
| 62 | Range | `$S62,{idx},0,0` | Range selection (0-21) |
| 63 | Gain | `$S63,{auto},{value},0,80,0` | Gain control (auto=0/1, value=0-100) |
| 64 | Sea | `$S64,{auto},{val},50,0,{mode},0` | Sea clutter (auto=0/1, val=0-100, mode: 0=Adv, 1=Coast) |
| 65 | Rain | `$S65,{auto},{value},0,0,0,0` | Rain clutter (auto=0/1, value=0-100) |
| 66 | CustomPictureAll | `$R66` | Request all picture settings |
| 67 | NoiseReduction | `$S67,0,3,{enabled},0` | Noise Reduction (0=OFF, 1=ON) |
| 68 | Unknown | `$N68,2,{idx},5/6,0,0` | Unknown (observed in responses) |
| 69 | Status | `$S69,{status},0,0,60,300,0` | Transmit (2) / Standby (1) |
| 6B | AcquireTarget | `$S6B,{x},{y},0` | Acquire target at coordinates |
| 6D | Unknown | `$N6D,1,1,0,0,30,0` | Unknown |
| 6E | Unknown | `$N6E,9,0,4,60,0,0,1` | Unknown |
| 70 | Unknown | `$N70,0,0,0` | Unknown |
| 73 | Unknown | `$R73,0,0` | Unknown |
| 74 | Unknown | `$N74,0,5,10,5000,600` | Unknown |
| 75 | Unknown | `$N75,1,1475,{idx}` | Unknown |
| 77 | BlindSector | `$S77,{idx},{start},{width},0,0` | Sector blanking (start + width in degrees) |
| 78 | Unknown | `$N78,0,0,0` | Unknown |
| 7A | Unknown | `$R7A,0` | Unknown |
| 7C | Unknown | `$R7C,0` | Unknown |
| 7D | Unknown | `$N7D,64543,-1,-1,-1` | Unknown |
| 7E | Unknown | `$R7E,0,0` | Unknown |
| 7F | Unknown | `$N7F,0,{idx}` | Unknown |
| 80 | Unknown | `$N80,1,32,0` | Unknown |
| 81 | HeadingAlign | `$S81,{deg*10},0` | Antenna heading alignment (0-3599, neg wraps: -1°=3590) |
| 82 | Unknown | `$N82,43,0,0,0,0` | Unknown |
| 83 | MainBangSize | `$S83,{val},0` | Main bang suppression (val=0-255, 0%=0, 100%=255) |
| 84 | AntennaHeight | `$S84,{cat},{m},0` | Antenna height (0,2=<3m; 1,5=3-10m; 2,15=>10m) |
| 85 | NearSTC | `$N85,2` | Near STC |
| 86 | MiddleSTC | `$R86,0` | Middle STC |
| 87 | FarSTC | `$R87,0` | Far STC |
| 88 | Unknown | `$N88,1` | Unknown |
| 89 | ScanSpeed | `$S89,{mode},0` | Antenna rotation (0=24rpm, 2=Auto) |
| 8A | Unknown | `$N8A,1` | Unknown |
| 8B-8D | Unknown | Various | Unknown |
| 8E | Unknown | `$N8E,83777400` | Large numeric value |
| 8F | Unknown | `$N8F,8302680` | Large numeric value |
| 90-9F | Unknown | Various | Unknown |
| A3 | Unknown | `$NA3,0,0,0,2,1,1,0,0` | Unknown |
| A6-AF | Unknown | Various | Unknown |
| AF | HeartbeatAck | `$NAF,256` | Periodic acknowledgment (~1s) |
| B0-C5 | Unknown | Various | Unknown settings |
| C6 | Unknown | `$NC6,150` | Unknown |
| C7 | Unknown | `$NC7,0,0` | Unknown |
| CA-D2 | Unknown | Various | Unknown |
| D3 | Commit | `$SD3,1,0` | Apply/commit command (sent after IntReject changes) |
| D4-D8 | Unknown | Various | Unknown |
| E0 | ClutterStatus | `$NE0,{idx},{auto},0,{value},0,0,0,1` | Clutter status report |
| E1-E2 | Unknown | Various | Unknown |
| E3 | KeepAlive | `$RE3` / `$NE3` | Keep-alive (send every ~5s) |
| E4-EB | Unknown | Various | Unknown |
| EC | TxChannel | `$SEC,{ch}` | TX Channel (0=Auto, 1-3=Ch1-3) |
| ED | BirdMode | `$SED,{level},0` | Bird Mode (0=OFF, 1=Low, 2=Med, 3=High) |
| EE | RezBoost | `$SEE,{level},0` | RezBoost (0=OFF, 1=Low, 2=Med, 3=High) |
| EF | TargetAnalyzer | `$SEF,{enabled},{mode},0` | Target Analyzer (mode: 0=Target, 1=Rain) |
| F0 | Unknown | `$NF0,1` | Unknown |
| F4-F5 | Status | `$NF5,{mode},{counter},0,0,0` | Periodic status (mode 3/4) |
| F8-FD | Unknown | Various | Unknown |
| FB | Unknown | `$NFB,24,13,5` | Unknown |
| FC | Unknown | `$NFC,1600,5,1,1` | Unknown |
| FD | Unknown | `$NFD,4,1,9,150,1,0,0` | Unknown |
| FF | Unknown | `$RFF,0,0` | Unknown |

### Key Commands for Control

#### Transmit/Standby (0x69)
```
$S69,{status},0,0,60,300,0\r\n
```
- status=1: Standby
- status=2: Transmit

Response:
```
$N69,{status},0,0,60,300,0\r\n
```

#### Range (0x62)
```
$S62,{range_index},0,0\r\n
```
Range indices (DRS4D-NXT):
| Index | Range |
|-------|-------|
| 21 | 1/16 nm (0.063) |
| 0 | 1/8 nm (0.125) |
| 1 | 1/4 nm (0.25) |
| 2 | 1/2 nm (0.5) |
| 3 | 3/4 nm (0.75) |
| 4 | 1 nm |
| 5 | 1.5 nm |
| 6 | 2 nm |
| 7 | 3 nm |
| 8 | 4 nm |
| 9 | 6 nm |
| 10 | 8 nm |
| 11 | 12 nm |
| 12 | 16 nm |
| 13 | 24 nm |
| 14 | 32 nm |
| 19 | 36 nm |
| 15 | 48 nm |

Note: Index 21 is minimum range, index 15 is maximum. Index 19 (36nm) is out of sequence.

#### Gain (0x63)
```
$S63,{auto},{value},0,80,0\r\n
```
- auto: 0=manual, 1=auto
- value: 0-100

Response includes both picture slots:
```
$N63,{auto},{value},0,80,0
$N63,{auto},{value},1,80,0
```

#### Sea Clutter (0x64)
```
$S64,{auto},{value},50,0,0,0\r\n
```
- auto: 0=manual, 1=auto
- value: 0-100 (second 50 appears to be a default)

#### Rain Clutter (0x65)
```
$S65,{auto},{value},0,0,0,0\r\n
```
- auto: 0=manual, 1=auto
- value: 0-100

#### RezBoost ($SEE)
```
$SEE,{level},0\r\n
```
| Level | Value |
|-------|-------|
| OFF | 0 |
| Low | 1 |
| Medium | 2 |
| High | 3 |

#### Signal Processing Settings (0x67)
Command 67 is a multi-purpose signal processing command. The 2nd parameter selects the feature:
```
$S67,0,{feature},{value},0\r\n
```

| Feature | Value | Description |
|---------|-------|-------------|
| 0 | 0/2 | Interference Rejection (0=OFF, 2=ON) |
| 3 | 0/1 | Noise Reduction (0=OFF, 1=ON) |

**Noise Reduction:**
```
$S67,0,3,{enabled},0\r\n
```
- enabled: 0=OFF, 1=ON

**Interference Rejection:**
```
$S67,0,0,{enabled},0\r\n
```
- enabled: 0=OFF, 2=ON
- Note: TimeZero also sends `$SD3,1,0` after this command (purpose unclear, possibly "apply/commit")

### Periodic Messages

The radar sends periodic messages without being requested:

1. **$NAF,256** - Every ~1 second (heartbeat acknowledgment)
2. **$NF5,{mode},{counter},0,0,0** - Status updates (mode 3 or 4, counter increments)
3. **$NE3** - Keep-alive response (after client sends $RE3)
4. **$N83,128,{level}** - Main bang size changes with range

### Keep-Alive Protocol

Client must send `$RE3\r\n` approximately every 5 seconds.
Radar responds with `$NE3\r\n`.

If keep-alive is not sent, the radar may disconnect the TCP session.

## DRS4D-NXT TCP Connection

**Important Finding**: The DRS4D-NXT **does** use TCP for control. Previous observations
that suggested UDP-only control were incorrect.

### Observed TCP Session (furuno4.pcap)

- Radar: 172.31.3.212
- Client: 172.31.3.152
- Command port: **10100** (not 10000 or 10001)

The session shows:
1. Client sends `$S69,1,0,0,60,300,0` (standby)
2. Radar responds `$N69,1,0,0,60,300,0`
3. Client sends `$S69,2,0,0,60,300,0` (transmit)
4. Radar responds `$N69,2,0,0,60,300,0`

### Why Earlier Attempts Failed

When capturing from a third-party machine (not the TimeZero PC), no TCP traffic
was visible because:

1. **TCP is point-to-point**: Unlike UDP broadcasts, TCP traffic only flows between
   the client (TimeZero PC) and radar
2. **Capture location**: Wireshark must run on the TimeZero PC itself, or the
   network must be configured for port mirroring
3. **Dynamic port**: The command port is not always 10001; it can be 10100 or other
   values based on the login response

### Connection Sequence

1. Client announces presence on UDP 10010
2. Client connects to TCP login port (10010 for DRS-NXT, 10000 for older)
3. Client sends login message with copyright string
4. Radar responds with command port offset
5. Client disconnects from login port
6. Client connects to TCP 10000 + offset (e.g., 10100)
7. Radar begins sending status messages
8. Client sends commands and receives responses
9. Client sends keep-alive every 5 seconds

## Verified Working Implementation

The mayara WASM plugin successfully controls the DRS4D-NXT radar via SignalK:

```
# Example session log:
[Furuno-RD003212] Starting login to 172.31.3.212:10010 (port idx 0)
[Furuno-RD003212] Login connection initiated to port 10010
[Furuno-RD003212] Login response: 12 bytes
[Furuno-RD003212] Got command port: 10100
[Furuno-RD003212] Connecting to command port 10100
[Furuno-RD003212] Command connection established
[Furuno-RD003212] Sending: $S69,2,0,0,60,300,0
[Furuno-RD003212] Response: $N69,2,0,0,60,300,0   <- Radar confirmed transmit!
```

The implementation handles:
- Multiple login ports (10010 first, then 10000)
- Fallback to direct command port connection (10100, 10001, 10002)
- Keep-alive every 5 seconds
- Automatic reconnection on disconnect

## Wireshark Capture Tips

### Display Filter for Commands
To capture only control commands (filtering out keepalive and data packets):
```
ip.addr == 172.31.3.212 && tcp.payload contains 24:53
```
This filters for packets containing `$S` (hex `24 53`), which is the command prefix.

### Alternative Filters
```
# All TCP traffic to/from radar
ip.addr == 172.31.3.212 && tcp

# Multiple radar IPs
(ip.addr == 172.31.3.212 || ip.addr == 172.31.3.54) && tcp.payload contains 24:53

# Command port range (if known)
tcp.port >= 10001 && tcp.port <= 10110 && tcp.len > 0
```

## Signal Processing Commands (0xEC-0xEF)

These commands control advanced signal processing features. They were decoded via Wireshark captures
from TimeZero Professional.

### TX Channel (0xEC)
```
$SEC,{channel}\r\n
```
| Value | Setting |
|-------|---------|
| 0 | Auto |
| 1 | Channel 1 |
| 2 | Channel 2 |
| 3 | Channel 3 |

Used to select transmission channel to avoid interference with other nearby radars.

### Bird Mode (0xED)
```
$SED,{level},{screen}\r\n
```
| Level | Setting |
|-------|---------|
| 0 | OFF |
| 1 | Low |
| 2 | Medium |
| 3 | High |

| Screen | Display |
|--------|---------|
| 0 | Primary |
| 1 | Secondary (dual scan) |

Optimizes radar display for detecting flocks of birds (useful for fishing). Note: Despite having a screen parameter, this appears to affect both screens (universal effect).

### RezBoost (0xEE)
```
$SEE,{level},{screen}\r\n
```
| Level | Setting |
|-------|---------|
| 0 | OFF |
| 1 | Low |
| 2 | Medium |
| 3 | High |

| Screen | Display |
|--------|---------|
| 0 | Primary |
| 1 | Secondary (dual scan) |

Resolution boost - enhances target separation and definition. Per-screen setting in dual scan mode.

### Target Analyzer (0xEF)
```
$SEF,{enabled},{mode},{screen}\r\n
```
| Enabled | Mode | Setting |
|---------|------|---------|
| 0 | - | OFF |
| 1 | 0 | Target mode |
| 1 | 1 | Rain mode |

| Screen | Display |
|--------|---------|
| 0 | Primary |
| 1 | Secondary (dual scan) |

Analyzes echoes to identify targets or rain patterns. Note: Despite having a screen parameter, this appears to affect both screens (universal effect).

## Antenna Settings

### Antenna Height (0x84)
```
$S84,{category},{meters},0\r\n
```
| Category | Meters | Height Range |
|----------|--------|--------------|
| 0 | 2 | Under 3m |
| 1 | 5 | 3-10m |
| 2 | 15 | Over 10m |

Antenna height affects sea clutter calculations.

### Heading Alignment (0x81)
```
$S81,{degrees_x10},0\r\n
```
- Value: 0-3599 (representing 0.0° to 359.9°)
- Negative values wrap: -1° = 3590, -2° = 3580
- Used to compensate for antenna mounting offset

### Scan Speed (0x89)
```
$S89,{mode},0\r\n
```
| Value | Setting |
|-------|---------|
| 0 | 24 RPM |
| 2 | Auto |

### Main Bang Suppression (0x83)
```
$S83,{value},0\r\n
```
- Value: 0-255 (linear mapping to 0-100%)
- Formula: percentage = value / 2.55
- Example: 50% = 128, 100% = 255

Suppresses the main bang (center reflection) on the radar display.

## ARPA Target Acquisition

### Acquire Target (0x6B)
```
$S6B,{x},{y},0\r\n
```
Coordinates for manual target acquisition. The x,y values are screen/spoke coordinates.

## Command Summary by Category

### Display & Control
| ID | Name | Description |
|----|------|-------------|
| 62 | Range | Range selection (0-21) |
| 63 | Gain | Gain control (0-100, auto) |
| 64 | Sea | Sea clutter (0-100, auto, mode) |
| 65 | Rain | Rain clutter (0-100, auto) |
| 69 | Status | Transmit/Standby |

### Signal Processing
| ID | Name | Description |
|----|------|-------------|
| 67 | Processing | Multi-purpose (IntReject, NoiseReduction) |
| EE | RezBoost | Resolution boost (OFF/Low/Med/High) |
| EF | TargetAnalyzer | Target/Rain analysis |
| ED | BirdMode | Bird detection (OFF/Low/Med/High) |

### Antenna
| ID | Name | Description |
|----|------|-------------|
| 77 | BlindSector | Sector blanking (no-transmit zones) |
| 81 | HeadingAlign | Heading offset (0-359.9°) |
| 83 | MainBang | Main bang suppression (0-100%) |
| 84 | AntennaHeight | Height category |
| 89 | ScanSpeed | Rotation speed |
| EC | TxChannel | TX channel selection |

### Sector Blanking / Blind Sector (0x77)
```
$S77,{s2_enable},{sector1_start},{sector1_width},{sector2_start},{sector2_width}\r\n
```

- `s2_enable`: Sector 2 enabled flag (0=sector 2 off, 1=sector 2 on)
- `sector1_start`: Sector 1 start angle in degrees (0-359)
- `sector1_width`: Sector 1 width in degrees (0 = sector 1 disabled)
- `sector2_start`: Sector 2 start angle in degrees (0-359)
- `sector2_width`: Sector 2 width in degrees

**Note**: The width parameters are sector **width**, not end angle.

To calculate width from UI start/end angles:
```
width = (end_angle - start_angle + 360) mod 360
```

**Examples:**

Sector 1 only (Start=200°, End=300°):
```
$S77,0,200,100,0,0
```

Both sectors (Sector 1: 200°-300°, Sector 2: 320°-20°):
```
$S77,1,200,100,320,60
```
(Sector 2 width: (20-320+360) mod 360 = 60°)

Disable both sectors:
```
$S77,0,0,0,0,0
```

Creates no-transmit zones where the radar won't transmit. Useful to avoid interference or protect areas.

### Targets
| ID | Name | Description |
|----|------|-------------|
| 6B | AcquireTarget | Manual ARPA target acquisition |
| F0 | AutoAcquire | ARPA auto acquire by Doppler |

### ARPA Auto Acquire (0xF0)
```
$SF0,{enabled}\r\n
```
| Value | Setting |
|-------|---------|
| 0 | OFF |
| 1 | ON (by Doppler) |

Enables automatic ARPA target acquisition based on Doppler detection. When enabled, the radar automatically tracks moving targets.

## Dual Scan Mode (Dual Range Display)

DRS-NXT radars support dual scan mode, allowing two independent radar displays with different ranges (up to 12nm each). Commands include a **screen index** parameter to target the specific display.

### Screen Index Parameter

The screen index identifies which radar display to control:
- `0` = Primary (1st) radar screen
- `1` = Secondary (2nd) radar screen

**Important**: The position of the screen parameter varies by command!

### Dual Scan Command Formats

| Command | Format | Screen Position |
|---------|--------|-----------------|
| 0x69 Status | `$S69,{status},{screen},0,60,300,0` | 3rd parameter |
| 0x62 Range | `$S62,{range},0,{screen}` | 4th parameter |

### Examples

**Transmit on 1st screen:**
```
$S69,2,0,0,60,300,0
```

**Transmit on 2nd screen:**
```
$S69,2,1,0,60,300,0
```

**Set range 3nm on 1st screen:**
```
$S62,7,0,0
```

**Set range 3nm on 2nd screen:**
```
$S62,7,0,1
```

### Dual Scan Limitations

- Maximum range for dual scan: 12nm (index 11)
- Both screens share the same antenna rotation

### Per-Screen vs Universal Settings

| Setting | Behavior |
|---------|----------|
| Range (0x62) | Per-screen (has screen index) |
| Status (0x69) | Per-screen (has screen index) |
| RezBoost (0xEE) | Per-screen (`$SEE,{level},{screen}`) |
| Gain (0x63) | Universal (affects both screens) |
| Sea clutter (0x64) | Universal (affects both screens) |
| Rain clutter (0x65) | Universal (affects both screens) |
| Bird Mode (0xED) | Universal (has screen param but affects both) |
| Target Analyzer (0xEF) | Universal (has screen param but affects both) |
| Int. Rejection (0x67) | Universal (affects both screens) |
| TX Channel (0xEC) | Universal (no screen param, single transmitter) |

## References

- mayara-lib source: `src/brand/furuno/`
- mayara-core protocol: `src/protocol/furuno/`
- Network captures:
  - `research/furuno/furuno_commands` - Complete command session dump
  - `/home/dirk/dev/furuno_pcap/furuno4.pcap` - TCP session with transmit/standby
- TimeZero Professional: https://mytimezero.com/tz-professional
- Protocol decoded via Wireshark analysis of TimeZero ↔ DRS4D-NXT communication
