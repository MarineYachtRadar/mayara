# DRS4D-NXT TCP Connection Investigation

Date: December 2024
**Status: RESOLVED** - TCP control works, see findings below.

## Summary

**The DRS4D-NXT radar DOES use TCP for control.** Earlier conclusions that it was
UDP-only were incorrect due to capturing from the wrong location.

### Key Findings

1. **TCP Control Port**: The command port is **dynamic** (e.g., 10100), not fixed at 10001
2. **Capture Location**: TCP traffic is only visible when capturing on the TimeZero PC itself
   (or via port mirroring), not from a third-party machine on the network
3. **Protocol**: Standard Furuno ASCII commands (`$S69,2,0,0,60,300,0` for transmit)
4. **Evidence**: furuno4.pcap shows TCP session between TimeZero PC and radar

### Working Command Examples

From furuno4.pcap:
```
Client → Radar: $S69,1,0,0,60,300,0    # Standby
Radar → Client: $N69,1,0,0,60,300,0    # Confirmed

Client → Radar: $S69,2,0,0,60,300,0    # Transmit
Radar → Client: $N69,2,0,0,60,300,0    # Confirmed
```

## Test Environment

- **Radar**: Furuno DRS4D-NXT at 172.31.3.212
- **TimeZero PC**: Windows 10 at 172.31.3.152 (in furuno4.pcap)
- **TimeZero Software**: TimeZero Professional (https://mytimezero.com/tz-professional)
- **Network**: Dedicated 172.31.0.0/16 subnet

### MAC Addresses
- Radar: `00:d0:1d:4a:be:bb` (Furuno OUI)
- TimeZero PC: Various (Hyper-V with MAC spoofing enabled)

## Experiments Conducted

### 1. TCP Connection Without MFD

**Test**: Stopped MFD, attempted TCP connection to radar port 10000.

**Result**: Connection refused (RST).
```
[172.31.3.212] 10000 (webmin) : Connection refused
```

**Conclusion**: The MFD is not blocking connections; the radar itself refuses TCP.

### 2. Announce Before TCP

**Test**: Sent UDP announce packets to 172.31.255.255:10010 from port 10010,
then attempted TCP connection.

**Result**: Connection still refused.

**Conclusion**: Announce packets alone are not sufficient.

### 3. MFD Network Traffic Analysis

**Test**: Captured all network traffic between MFD and radar during standby/transmit
toggle operations.

**Result**:
- MFD sends NO TCP traffic to the radar
- MFD only sends UDP broadcasts:
  - Port 10010: Beacon (36, 16, 170 bytes)
  - Port 10021: NMEA position ($IIGGA, $IIVTG)
  - Port 10036: NMEA heading ($IIHDT)
  - Port 33000: TZ Sync
- No unicast packets from MFD to radar (only ARP)

**Conclusion**: The MFD controls the radar without TCP. Either:
1. Control is embedded in UDP packets
2. Control uses an undiscovered protocol
3. Control is via the radar's physical buttons/menus and MFD just displays

### 4. Beacon Packet Analysis

The 170-byte beacon packets contain MAC addresses:

**MFD Beacon** (from 172.31.3.54):
```
Offset 0x26-0x2B: 70:85:c2:a7:a2:35
Name: MXT0(TABLETPC)
```

**Radar Beacon** (from 172.31.3.212):
```
Offset 0x26-0x2B: 00:d0:1d:4a:be:bb
Name: DRS4D-NXT
Firmware: 01.01:01.01:01.05:01.05
Serial: 6424
```

### 5. Research File Analysis

The `research/furuno/furuno_commands` file contains a captured TCP session
showing the radar DOES accept TCP and respond to commands:

```
$S69,2,0,0,60,300,0    <- Transmit command
$N69,2,0,0,60,300,0    <- Radar confirms
$S62,4,0,0             <- Range index 4
$N62,4,0,0             <- Radar confirms
```

**Conclusion**: TCP works when captured from a working connection. The question
is what prerequisites enable the TCP connection.

## Hypotheses

### H1: MAC Address Whitelist

The radar may only accept TCP connections from devices with:
- Furuno MAC OUI (00:d0:1d:xx:xx:xx)
- MAC addresses that have been "paired" via some registration protocol
- MAC addresses advertised in beacon packets from the same port (10010)

**Evidence for**:
- MAC address is embedded in beacon packets
- Radar has Furuno OUI
- MFD has non-Furuno MAC but controls radar (via UDP, not TCP?)

**Evidence against**:
- MFD does not use TCP at all in our captures

### H2: UDP-Only Control for DRS-NXT

The DRS-NXT series may use a different (UDP-based) control protocol than
the FAR series. The TCP protocol may only be used for:
- Initial pairing/registration
- Firmware updates
- Service diagnostics

**Evidence for**:
- MFD has no TCP traffic in captures
- DRS-NXT is consumer-grade, FAR is commercial-grade

**Evidence against**:
- `furuno_commands` file shows TCP session with DRS commands

### H3: TZ Sync Protocol (Port 33000)

The control commands may be embedded in the TimeZero Sync protocol which
broadcasts on port 33000 with 164-byte packets containing:
```
TZ Sync 1.0;MASTERCABIN;TZ Professional;...
```

This would require reverse-engineering the TZ Sync format.

### H4: Hardware Button Control

The MFD may not control the radar at all via network. The radar may be
controlled via:
- Physical buttons on the radar dome
- The TZT3's touchscreen sending signals via a different mechanism
- A wired connection (CAN bus, etc.)

**Evidence against**:
- Radar name in beacon includes "(TABLETPC)" suggesting network control

### 6. MFD Transmit/Standby Toggle Capture (December 10, 2024)

**Test**: Captured full network traffic while MFD toggled transmit/standby 3 times.

**Result**:

1. **NO unicast packets to radar**: All MFD traffic is broadcast UDP.

2. **Port 10010 beacon burst during toggle**:
   The MFD sends a burst of beacon packets when the user touches transmit/standby:
   ```
   Command 0x01: Beacon request (16 bytes)
   Command 0x14: Model info request (16 bytes)
   Command 0x15: Unknown request (16 bytes)
   Command 0x18: Unknown request (16 bytes)
   Command 0x1b: Status with MAC (40 bytes) - contains MFD MAC + flags
   Command 0x17: Identify with MAC (25 bytes)
   ```

3. **Packets are IDENTICAL between toggles**: The same exact packet sequence
   is sent for each toggle - no transmit/standby state is encoded.

4. **Radar responses unchanged**: The radar's 170-byte model info response is
   identical before and after toggle.

5. **No control commands found**: Searched all ports and packet types. The
   transmit/standby command is not sent via network.

**Conclusion**: **H4 confirmed** - The TZT3 MFD does NOT control the radar
via the Ethernet network. The TZT3 is an integrated MFD that runs TimeZero
internally (the device at 172.31.3.54 IS TimeZero, not a separate PC).

The radar has ONLY Ethernet connectivity (no CAN bus/N2K), so control must be
via one of:
- Internal bus within the TZT3 to its radar processor unit
- A separate proprietary digital cable between TZT3 and radar dome
- A protocol we haven't discovered (though we captured ALL traffic)

The Ethernet network is used only for:
- Data display (spokes on port 10024, ARPA on 10034)
- Position/heading sync (ports 10021, 10036)
- Device discovery (port 10010)

### 7. Beacon Protocol Command Reference

| ID | Size | Description |
|----|------|-------------|
| 0x00 | 36 | Device announce with name (MF003054, RD003212) |
| 0x01 | 16 | Beacon request |
| 0x0f | 170 | Model info response (MAC, name, firmware, serial) |
| 0x14 | 16 | Model info request |
| 0x15 | 16 | Unknown request |
| 0x17 | 25 | Identify with MAC address |
| 0x18 | 16 | Unknown request |
| 0x1b | 40 | Status with MAC address + flags |

### 8. Key Correction: TimeZero Professional on Windows PC

172.31.3.54 is a **Windows PC running TimeZero Professional** (not an integrated
MFD). The PC has Hyper-V with a VM switch configured for the Furuno subnet with
MAC spoofing and multicast enabled.

This is critical: **ALL control traffic is UDP**. No TCP is used anywhere.
The control commands MUST be in the UDP packets somewhere, but we have not yet
identified where the transmit/standby command is encoded.

Observed UDP traffic from TimeZero PC:
- Port 10010: Beacon/discovery (16, 36, 40, 82, 170, 212 byte packets)
- Port 10021: NMEA position ($IIGGA, $IIVTG)
- Port 10036: NMEA heading ($IIHDT)
- Port 33000: TZ Sync (159 bytes)

The beacon packets appear IDENTICAL during toggle operations. Either:
- The control is in a field we haven't decoded yet
- The control uses a different mechanism (possibly embedded in TZ Sync)
- The capture missed something

## Resolution

**RESOLVED**: The DRS4D-NXT radar uses standard Furuno TCP control protocol.

### Why Earlier Attempts Failed

1. **Capture location was wrong**: Capturing from a third-party machine on the network
   only shows broadcast UDP traffic. TCP is point-to-point and only visible:
   - On the TimeZero PC itself (running Wireshark locally)
   - Via port mirroring on the network switch

2. **Dynamic command port**: The radar assigns a dynamic command port (observed: 10100)
   during login, not the fixed port 10001 documented elsewhere.

3. **Connection refusal from wrong client**: The radar may only accept TCP connections
   from clients that have properly announced via UDP beacon protocol first. The exact
   requirements are still being investigated.

### Next Steps for mayara

1. **Implement TCP login sequence**:
   - Send UDP announce on port 10010
   - Connect to TCP port 10000
   - Send login message with copyright string
   - Parse response for command port offset
   - Connect to command port (10000 + offset)

2. **Implement command protocol**:
   - Send `$S69,2,0,0,60,300,0` for transmit
   - Send `$S69,1,0,0,60,300,0` for standby
   - Send `$S62,{idx},0,0` for range
   - Send `$RE3` every 5 seconds for keep-alive

3. **Handle responses**:
   - Parse `$N{cmd},...` responses
   - Handle periodic `$NAF,256` and `$NF5,...` status messages

See `docs/furuno/protocol.md` for complete command reference.

## Files

- **TCP session capture**: `/home/dirk/dev/furuno_pcap/furuno4.pcap` (646KB)
- **Command dump**: `research/furuno/furuno_commands` (complete session)
- **Protocol docs**: `docs/furuno/protocol.md`
- Earlier captures (no TCP, captured from wrong location):
  - `/tmp/standby_capture.pcap`
  - `/home/dirk/dev/furuno_pcap/furuno1-3.pcap`
