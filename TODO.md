### TODO.md

Working:

* Start, logging
* Detect BR24, 3G, 4G and HALO radars
* Detect Raymarine radars (tested with Quantum 2)
* Detect Furuno radars (tested with DRS4D-NXT)
* Provide webserver for static and dynamic pages
* Serve Navico and Furuno radar data
* Control Navico radar (tested with 4G and HALO)
* Trails in relative mode
* Getting heading and location from Signal K or NMEA 0183 server

Work in Progress:

* Target acquisition (M)ARPA
* Detect Garmin xHD (but not yet know if different from HD)
* Furuno control - implemented for DRS4D-NXT (except ARPA)

TODO:

* Guard zones
* Everything else


### dirkwa

Webgui: Network Configuration help autocolaps every 1-2 sec even when no radar is found. 


- API 
  - What sees Client?
  - Descriptions
  - Default
  - info 
    - firmware
    - hours of operation 


files the same? WASM/Standalone?
- locator rs
- Build rs
- command rs?

- furuno controller ?


Does the model database have the functions?
PARTIALLY - The ModelInfo struct in mayara-core has:
✅ Range tables, spokes, characteristics
✅ Feature flags (has_doppler, has_dual_range)
✅ Control IDs list (controls: &'static [&'static str])
❌ NO actual control command implementations - those are in mayara-lib only
3. Same registration to SignalK?
NO - They are different:
mayara-lib + mayara-server: Uses its own REST API (/v1/api/radars), not SignalK
mayara-signalk-wasm: Registers via sk_register_radar_provider FFI, uses SignalK's radar API
They don't share registration code - they're two different architectures.

- raymarin, halo, etc just stubs by now in core. 
- rest api same as SignalK? for standalone?


- build warnings?
- Add WebGPU view for standalone
- Update all docs
- Update Readme
-- Link Wasm readme

- bundled mode --> Standalone talking to SK-API

--> Mayara WASAM : Provider and Plugin with it's web gui. 
Can we modify the webapp part in /public that it subscribes as a client (like Chartplotter) to the new radar api and change the control panel that based upon the capapbilities of the radar detected it will offer controls and map them to be usable? I prefer sliders and buttons, no drop down. rhould be touch fiendly. curent CSS is quite nice. 
