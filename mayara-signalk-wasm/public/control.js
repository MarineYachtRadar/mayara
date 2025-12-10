export { loadRadar, registerRadarCallback, registerControlCallback, setCurrentRange };

import van from "./van-1.5.2.js";
import { fetchRadars, detectMode, sendControlCommand } from "./api.js";

const { div, label, input, button, select, option, span } = van.tags;

const prefix = "myr_";
const auto_postfix = "_auto";
const enabled_postfix = "_enabled";

const RANGE_UNIT_SELECT_ID = 999;

// Furuno DRS4D-NXT range table (meters, sorted by distance)
// Verified via Wireshark captures from TimeZero ↔ DRS4D-NXT
// Note: Wire indices are non-sequential - use RANGE_WIRE_INDEX for protocol
const RANGE_TABLE = [
  116,    // 0: 1/16 nm (wire index 21)
  231,    // 1: 1/8 nm (wire index 0)
  463,    // 2: 1/4 nm (wire index 1)
  926,    // 3: 1/2 nm (wire index 2)
  1389,   // 4: 3/4 nm (wire index 3)
  1852,   // 5: 1 nm (wire index 4)
  2778,   // 6: 1.5 nm (wire index 5)
  3704,   // 7: 2 nm (wire index 6)
  5556,   // 8: 3 nm (wire index 7)
  7408,   // 9: 4 nm (wire index 8)
  11112,  // 10: 6 nm (wire index 9)
  14816,  // 11: 8 nm (wire index 10)
  22224,  // 12: 12 nm (wire index 11)
  29632,  // 13: 16 nm (wire index 12)
  44448,  // 14: 24 nm (wire index 13)
  59264,  // 15: 32 nm (wire index 14)
  66672,  // 16: 36 nm (wire index 19 - out of sequence!)
  88896,  // 17: 48 nm (wire index 15 - max for DRS4D-NXT)
];

// Wire protocol indices (array index here → wire index to send)
// Maps RANGE_TABLE array index to Furuno protocol wire index
const RANGE_WIRE_INDEX = [
  21,  // 1/16 nm
  0,   // 1/8 nm
  1,   // 1/4 nm
  2,   // 1/2 nm
  3,   // 3/4 nm
  4,   // 1 nm
  5,   // 1.5 nm
  6,   // 2 nm
  7,   // 3 nm
  8,   // 4 nm
  9,   // 6 nm
  10,  // 8 nm
  11,  // 12 nm
  12,  // 16 nm
  13,  // 24 nm
  14,  // 32 nm
  19,  // 36 nm (out of sequence!)
  15,  // 48 nm
];

var myr_radar;
var myr_controls;
var myr_range_control_id;
var myr_current_range = 1852; // Default 1nm
var myr_webSocket;
var myr_error_message;
var myr_no_response_timeout;
var myr_callbacks = Array();
var myr_control_callbacks = Array();

function registerRadarCallback(callback) {
  myr_callbacks.push(callback);
}

function registerControlCallback(callback) {
  myr_control_callbacks.push(callback);
}

const ReadOnlyValue = (id, name) =>
  div(
    { class: "myr_control myr_readonly" },
    div(name),
    div({ class: "myr_numeric", id: prefix + id })
  );

const StringValue = (id, name) =>
  div(
    { class: "myr_control" },
    label({ for: prefix + id }, name),
    input({ type: "text", id: prefix + id, size: 20 })
  );

const NumericValue = (id, name) =>
  div(
    { class: "myr_control" },
    div({ class: "myr_numeric" }),
    label({ for: prefix + id }, name),
    input({
      type: "number",
      id: prefix + id,
      onchange: (e) => do_change(e),
      oninput: (e) => do_input(e),
    })
  );

const RangeValue = (id, name, min, max, def, descriptions) =>
  div(
    { class: "myr_control" },
    div({ class: "myr_description" }),
    label({ for: prefix + id }, name),
    input({
      type: "range",
      id: prefix + id,
      min,
      max,
      value: def,
      onchange: (e) => do_change(e),
    })
  );

const ButtonValue = (id, name) =>
  div(
    { class: "myr_button" },
    button(
      { type: "button", id: prefix + id, onclick: (e) => do_change(e) },
      name
    )
  );

const PowerButton = (state, label, isActive) =>
  button(
    {
      type: "button",
      class: `myr_power_button ${isActive ? "myr_power_active" : ""}`,
      onclick: () => sendPowerCommand(state),
    },
    label
  );

function sendPowerCommand(state) {
  if (!myr_radar) return;

  // Find the power control ID (typically "1")
  let powerControlId = null;
  for (const [k, v] of Object.entries(myr_controls)) {
    if (v.name === "Power") {
      powerControlId = k;
      break;
    }
  }

  if (powerControlId) {
    const message = { id: powerControlId, value: state };
    const cv = JSON.stringify(message);
    sendControlMessage(cv, "Power");

    // Update button states immediately for responsiveness
    updatePowerButtonStates(state);
  }
}

function updatePowerButtonStates(activeState) {
  const transmitBtn = document.querySelector(".myr_power_button_transmit");
  const standbyBtn = document.querySelector(".myr_power_button_standby");

  if (transmitBtn) {
    transmitBtn.classList.toggle("myr_power_active", activeState === "transmit");
  }
  if (standbyBtn) {
    standbyBtn.classList.toggle("myr_power_active", activeState === "standby");
  }
}

function buildPowerButtons(container) {
  const currentStatus = myr_radar.status || "standby";

  const powerDiv = div(
    { class: "myr_power_buttons" },
    button(
      {
        type: "button",
        class: `myr_power_button myr_power_button_transmit ${currentStatus === "transmit" ? "myr_power_active" : ""}`,
        onclick: () => sendPowerCommand("transmit"),
      },
      "Transmit"
    ),
    button(
      {
        type: "button",
        class: `myr_power_button myr_power_button_standby ${currentStatus === "standby" ? "myr_power_active" : ""}`,
        onclick: () => sendPowerCommand("standby"),
      },
      "Standby"
    )
  );

  van.add(container, powerDiv);
}

// Find closest range index in RANGE_TABLE
function findRangeIndex(meters) {
  let closest = 0;
  let minDiff = Math.abs(RANGE_TABLE[0] - meters);
  for (let i = 1; i < RANGE_TABLE.length; i++) {
    const diff = Math.abs(RANGE_TABLE[i] - meters);
    if (diff < minDiff) {
      minDiff = diff;
      closest = i;
    }
  }
  return closest;
}

// Format range value for display
function formatRange(meters) {
  const nm = meters / 1852;
  if (nm >= 1) {
    // Handle fractional nm values like 1.5
    if (nm % 1 !== 0 && nm < 2) {
      return "1.5 nm";
    }
    return Math.round(nm) + " nm";
  } else if (nm >= 0.7) {
    return "3/4 nm";
  } else if (nm >= 0.4) {
    return "1/2 nm";
  } else if (nm >= 0.2) {
    return "1/4 nm";
  } else if (nm >= 0.1) {
    return "1/8 nm";
  } else {
    return "1/16 nm";
  }
}

// Send range command via REST API
async function sendRangeCommand(meters) {
  if (!myr_radar) return;

  const url = `/signalk/v2/api/vessels/self/radars/${myr_radar.id}/range`;
  const body = { value: meters };

  console.log(`Range command: PUT ${url}`, body);

  try {
    const response = await fetch(url, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });

    if (response.ok) {
      console.log(`Range set to ${meters}m`);
      myr_current_range = meters;
      updateRangeDisplay();
    } else {
      console.error(`Range command failed: ${response.status}`);
    }
  } catch (e) {
    console.error(`Range command error: ${e}`);
  }
}

function rangeUp() {
  const currentIndex = findRangeIndex(myr_current_range);
  console.log(`rangeUp: current=${myr_current_range}m, index=${currentIndex}, next=${currentIndex + 1}, nextValue=${RANGE_TABLE[currentIndex + 1]}m`);
  if (currentIndex < RANGE_TABLE.length - 1) {
    sendRangeCommand(RANGE_TABLE[currentIndex + 1]);
  }
}

function rangeDown() {
  const currentIndex = findRangeIndex(myr_current_range);
  console.log(`rangeDown: current=${myr_current_range}m, index=${currentIndex}, prev=${currentIndex - 1}, prevValue=${RANGE_TABLE[currentIndex - 1]}m`);
  if (currentIndex > 0) {
    sendRangeCommand(RANGE_TABLE[currentIndex - 1]);
  }
}

function updateRangeDisplay() {
  const display = document.getElementById("myr_range_display");
  if (display) {
    display.textContent = formatRange(myr_current_range);
  }
}

// Called from viewer.js when spoke data contains range
function setCurrentRange(meters) {
  if (meters > 0 && meters !== myr_current_range) {
    const tableIndex = findRangeIndex(meters);
    console.log(`setCurrentRange: radar reports ${meters}m, table index ${tableIndex} (${formatRange(meters)})`);
    myr_current_range = meters;
    updateRangeDisplay();
  }
}

function buildRangeButtons(container) {
  const rangeDiv = div(
    { class: "myr_range_buttons" },
    button(
      {
        type: "button",
        class: "myr_range_button",
        onclick: () => rangeDown(),
      },
      "Range -"
    ),
    button(
      {
        type: "button",
        class: "myr_range_button",
        onclick: () => rangeUp(),
      },
      "Range +"
    )
  );

  van.add(container, rangeDiv);
}

// Gain/Sea/Rain slider state - defaults for Furuno DRS4D-NXT
var myr_gain_value = 50;
var myr_gain_auto = true;   // Gain defaults to auto
var myr_sea_value = 50;
var myr_sea_auto = true;    // Sea defaults to auto
var myr_rain_value = 0;     // Rain defaults to 0 (off)
var myr_rain_auto = false;

// Send gain command via REST API
async function sendGainCommand(value, auto) {
  if (!myr_radar) return;

  const url = `/signalk/v2/api/vessels/self/radars/${myr_radar.id}/gain`;
  const body = { auto: auto, value: auto ? undefined : value };

  console.log(`Gain command: PUT ${url}`, body);

  try {
    const response = await fetch(url, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });

    if (response.ok) {
      console.log(`Gain set: auto=${auto}, value=${value}`);
      myr_gain_value = value;
      myr_gain_auto = auto;
    } else {
      console.error(`Gain command failed: ${response.status}`);
    }
  } catch (e) {
    console.error(`Gain command error: ${e}`);
  }
}

// Send sea command via REST API
async function sendSeaCommand(value, auto) {
  if (!myr_radar) return;

  const url = `/signalk/v2/api/vessels/self/radars/${myr_radar.id}/sea`;
  const body = { auto: auto, value: auto ? undefined : value };

  console.log(`Sea command: PUT ${url}`, body);

  try {
    const response = await fetch(url, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });

    if (response.ok) {
      console.log(`Sea set: auto=${auto}, value=${value}`);
      myr_sea_value = value;
      myr_sea_auto = auto;
    } else {
      console.error(`Sea command failed: ${response.status}`);
    }
  } catch (e) {
    console.error(`Sea command error: ${e}`);
  }
}

// Send rain command via REST API
async function sendRainCommand(value, auto) {
  if (!myr_radar) return;

  const url = `/signalk/v2/api/vessels/self/radars/${myr_radar.id}/rain`;
  const body = { auto: auto, value: auto ? undefined : value };

  console.log(`Rain command: PUT ${url}`, body);

  try {
    const response = await fetch(url, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });

    if (response.ok) {
      console.log(`Rain set: auto=${auto}, value=${value}`);
      myr_rain_value = value;
      myr_rain_auto = auto;
    } else {
      console.error(`Rain command failed: ${response.status}`);
    }
  } catch (e) {
    console.error(`Rain command error: ${e}`);
  }
}

function buildClutterControls(container) {
  // Gain control
  const gainDiv = div(
    { class: "myr_clutter_control" },
    div(
      { class: "myr_clutter_header" },
      span({ class: "myr_clutter_label" }, "Gain"),
      label({ class: "myr_auto_checkbox" },
        input({
          type: "checkbox",
          id: "myr_gain_auto",
          checked: myr_gain_auto,
          onchange: (e) => {
            myr_gain_auto = e.target.checked;
            const slider = document.getElementById("myr_gain_slider");
            slider.disabled = myr_gain_auto;
            sendGainCommand(myr_gain_value, myr_gain_auto);
          },
        }),
        " Auto"
      )
    ),
    input({
      type: "range",
      id: "myr_gain_slider",
      class: "myr_clutter_slider",
      min: 0,
      max: 100,
      value: myr_gain_value,
      disabled: myr_gain_auto,
      onchange: (e) => {
        myr_gain_value = parseInt(e.target.value);
        sendGainCommand(myr_gain_value, myr_gain_auto);
      },
    })
  );

  // Sea control
  const seaDiv = div(
    { class: "myr_clutter_control" },
    div(
      { class: "myr_clutter_header" },
      span({ class: "myr_clutter_label" }, "Sea"),
      label({ class: "myr_auto_checkbox" },
        input({
          type: "checkbox",
          id: "myr_sea_auto",
          checked: myr_sea_auto,
          onchange: (e) => {
            myr_sea_auto = e.target.checked;
            const slider = document.getElementById("myr_sea_slider");
            slider.disabled = myr_sea_auto;
            sendSeaCommand(myr_sea_value, myr_sea_auto);
          },
        }),
        " Auto"
      )
    ),
    input({
      type: "range",
      id: "myr_sea_slider",
      class: "myr_clutter_slider",
      min: 0,
      max: 100,
      value: myr_sea_value,
      disabled: myr_sea_auto,
      onchange: (e) => {
        myr_sea_value = parseInt(e.target.value);
        sendSeaCommand(myr_sea_value, myr_sea_auto);
      },
    })
  );

  // Rain control
  const rainDiv = div(
    { class: "myr_clutter_control" },
    div(
      { class: "myr_clutter_header" },
      span({ class: "myr_clutter_label" }, "Rain"),
      label({ class: "myr_auto_checkbox" },
        input({
          type: "checkbox",
          id: "myr_rain_auto",
          checked: myr_rain_auto,
          onchange: (e) => {
            myr_rain_auto = e.target.checked;
            const slider = document.getElementById("myr_rain_slider");
            slider.disabled = myr_rain_auto;
            sendRainCommand(myr_rain_value, myr_rain_auto);
          },
        }),
        " Auto"
      )
    ),
    input({
      type: "range",
      id: "myr_rain_slider",
      class: "myr_clutter_slider",
      min: 0,
      max: 100,
      value: myr_rain_value,
      disabled: myr_rain_auto,
      onchange: (e) => {
        myr_rain_value = parseInt(e.target.value);
        sendRainCommand(myr_rain_value, myr_rain_auto);
      },
    })
  );

  van.add(container, gainDiv);
  van.add(container, seaDiv);
  van.add(container, rainDiv);
}

const AutoButton = (id) =>
  div(
    { class: "myr_button" },
    label({ for: prefix + id + auto_postfix, class: "myr_auto_label" }, "Auto"),
    input({
      type: "checkbox",
      class: "myr_auto",
      id: prefix + id + auto_postfix,
      onchange: (e) => do_change_auto(e),
    })
  );

const EnabledButton = (id) =>
  div(
    { class: "myr_button" },
    label(
      { for: prefix + id + enabled_postfix, class: "myr_enabled_label" },
      "Enabled"
    ),
    input({
      type: "checkbox",
      class: "myr_enabled",
      id: prefix + id + enabled_postfix,
      onchange: (e) => do_change_enabled(e),
    })
  );

const SelectValue = (id, name, validValues, descriptions) => {
  let r = div(
    { class: "myr_control" },
    label({ for: prefix + id }, name),
    div({ class: "myr_description" }),
    select(
      { id: prefix + id, onchange: (e) => do_change(e) },
      validValues.map((v) => option({ value: v }, descriptions[v]))
    )
  );
  return r;
};

const SetButton = () =>
  button({ type: "button", onclick: (e) => do_button(e) }, "Set");

class TemporaryMessage {
  timeoutId;
  element;

  constructor(id) {
    this.element = get_element_by_server_id(id);
  }

  raise(aMessage) {
    this.element.style.visibility = "visible";
    this.element.classList.remove("myr_vanish");
    this.element.innerHTML = aMessage;
    this.timeoutId = setTimeout(() => {
      this.cancel();
    }, 5000);
  }

  cancel() {
    if (typeof this.timeoutId === "number") {
      clearTimeout(this.timeoutId);
    }
    this.element.classList.add("myr_vanish");
  }
}

class Timeout {
  timeoutId;
  element;

  constructor(id) {
    this.element = get_element_by_server_id(id);
  }

  setTimeout() {
    this.cancel();
    this.timeoutId = setTimeout(() => {
      setControl({ id: "0", value: "0" });
    }, 15000);
  }

  cancel() {
    if (typeof this.timeoutId === "number") {
      clearTimeout(this.timeoutId);
      this.timeoutId = undefined;
    }
  }
}

//
// This is not called when used in a nested module.
//
window.onload = function () {
  const urlParams = new URLSearchParams(window.location.search);
  const id = urlParams.get("id");

  loadRadar(id);
};

async function loadRadar(id) {
  try {
    await detectMode();
    const radars = await fetchRadars();
    radarsLoaded(id, radars);
  } catch (err) {
    console.error("Failed to load radars:", err);
    restart(id);
  }
}

function restart(id) {
  console.log("restart(" + id + ")");
  setTimeout(loadRadar, 15000, id);
}

function radarsLoaded(id, d) {
  myr_radar = d[id];

  if (myr_radar === undefined || myr_radar.controls === undefined) {
    restart(id);
    return;
  }
  myr_controls = myr_radar.controls;
  myr_error_message = new TemporaryMessage("error");

  buildControls();
  myr_no_response_timeout = new Timeout("0");

  // Only connect control WebSocket if controlUrl is explicitly provided
  // (SignalK WASM plugins use REST API for controls, not WebSocket)
  let controlUrl = myr_radar.controlUrl;
  if (controlUrl) {
    myr_webSocket = new WebSocket(controlUrl);

    myr_webSocket.onopen = (e) => {
      console.log("control websocket open: " + JSON.stringify(e));
    };
    myr_webSocket.onclose = (e) => {
      console.log("control websocket close: " + e);
      let v = { id: "0", value: "0" };
      setControl(v);
      restart(id);
    };
    myr_webSocket.onmessage = (e) => {
      let v = JSON.parse(e.data);
      console.log("<- " + e.data);
      setControl(v);
      myr_no_response_timeout.setTimeout();
    };
  } else {
    console.log("No controlUrl provided - controls via REST API only");
  }

  myr_callbacks.forEach((cb) => {
    cb(myr_radar);
  });
}

function setControl(v) {
  let i = get_element_by_server_id(v.id);
  let control = myr_controls[v.id];
  if (i && control) {
    i.value = v.value;
    console.log("<- " + control.name + " = " + v.value);
    let n = i.parentNode.querySelector(".myr_numeric");
    if (n) {
      if (control.unit) {
        n.innerHTML = v.value + " " + control.unit;
      } else {
        n.innerHTML = v.value;
      }
    }
    let d = i.parentNode.querySelector(".myr_description");
    if (d) {
      let description = control.descriptions
        ? control.descriptions[v.value]
        : undefined;
      if (!description && control.hasAutoAdjustable) {
        if (v["auto"]) {
          description =
            "A" +
            (v.value > 0 ? "+" + v.value : "") +
            (v.value < 0 ? v.value : "");
          i.min = control.autoAdjustMinValue;
          i.max = control.autoAdjustMaxValue;
        } else {
          i.min = control.minValue;
          i.max = control.maxValue;
        }
      }
      if (!description) description = v.value;
      d.innerHTML = description;
    }

    if (control.hasAuto && "auto" in v) {
      let checkbox = i.parentNode.querySelector(".myr_auto");
      if (checkbox) {
        checkbox.checked = v.auto;
      }
      let display = v.auto && !control.hasAutoAdjustable ? "none" : "block";
      if (n) {
        n.style.display = display;
      }
      if (d) {
        d.style.display = display;
      }
      i.style.display = display;
    }

    if ("enabled" in v) {
      let checkbox = i.parentNode.querySelector(".myr_enabled");
      if (checkbox) {
        checkbox.checked = v.enabled;
      }
      let display = v.enabled ? "block" : "none";
      if (n) {
        n.style.display = display;
      }
      if (d) {
        d.style.display = display;
      }
      i.style.display = display;
    }

    if (control.name == "Range") {
      myr_range_control_id = v.id;

      let r = parseFloat(v.value);
      if (control.descriptions && control.descriptions[r]) {
        let unit = control.descriptions[r].split(/(\s+)/);
        // Filter either on 'nm' or 'm'
        if (unit.length == 3) {
          let units = get_element_by_server_id(RANGE_UNIT_SELECT_ID);
          if (units) {
            let new_value = unit[2] == "nm" ? 1 : 0;
            if (units.value != new_value) {
              // Only change if different
              units.value = new_value;
              handle_range_unit_change(new_value);
              i.value = v.value;
            }
          }
        }
      }
    }

    // Update power buttons when Status or Power control changes
    if (control.name === "Status" || control.name === "Power") {
      const state = String(v.value).toLowerCase();
      if (state === "transmit" || state === "standby") {
        updatePowerButtonStates(state);
      }
    }

    myr_control_callbacks.forEach((cb) => {
      cb(control, v);
    });

    if (v.error) {
      myr_error_message.raise(v.error);
    }
  }
}

function buildControls() {
  let c = get_element_by_server_id("title");
  c.innerHTML = "";
  van.add(c, div(myr_radar.name + " Controls"));

  c = get_element_by_server_id("controls");
  c.innerHTML = "";

  // Add power buttons at the top
  buildPowerButtons(c);

  // Add range +/- buttons
  buildRangeButtons(c);

  // Add gain/sea/rain sliders
  buildClutterControls(c);
}

function add_range_unit_select(c, descriptions) {
  let found_metric = false;
  let found_nautical = false;
  for (const [k, v] of Object.entries(descriptions)) {
    if (v.match(/ nm$/)) {
      found_nautical = true;
    } else {
      found_metric = true;
    }
  }
  if (found_metric && found_nautical) {
    van.add(
      c,
      SelectValue(RANGE_UNIT_SELECT_ID, "Range units", [0, 1], {
        0: "Metric",
        1: "Nautic",
      })
    );
  }
}

function do_change(e) {
  let v = e.target;
  let id = html_to_server_id(v.id);
  console.log("change " + e + " " + id + "=" + v.value);
  if (id == RANGE_UNIT_SELECT_ID) {
    handle_range_unit_change(v.value);
    return;
  }
  let message = { id: id, value: v.value };
  let checkbox = document.getElementById(v.id + auto_postfix);
  if (checkbox) {
    message.auto = checkbox.checked;
  }
  checkbox = document.getElementById(v.id + enabled_postfix);
  if (checkbox) {
    message.enabled = checkbox.checked;
  }
  let cv = JSON.stringify(message);
  sendControlMessage(cv, myr_controls[id].name);
}

function sendControlMessage(cv, controlName) {
  if (myr_webSocket && myr_webSocket.readyState === WebSocket.OPEN) {
    myr_webSocket.send(cv);
    console.log(controlName + "-> " + cv);
  } else {
    // Use REST API when WebSocket not available
    const controlData = JSON.parse(cv);
    console.log(controlName + " (REST)-> " + cv);
    sendControlCommand(myr_radar.id, controlData, myr_controls);
  }
}

function do_change_auto(e) {
  let checkbox = e.target;
  let id = html_to_server_id(checkbox.id);
  let v = document.getElementById(html_to_value_id(checkbox.id));
  console.log(
    "change auto " + e + " " + id + "=" + v.value + " auto=" + checkbox.checked
  );
  let cv = JSON.stringify({ id: id, value: v.value, auto: checkbox.checked });
  sendControlMessage(cv, myr_controls[id].name);
}

function do_change_enabled(e) {
  let checkbox = e.target;
  let id = html_to_server_id(checkbox.id);
  let v = document.getElementById(html_to_value_id(checkbox.id));
  console.log(
    "change enabled " +
      e +
      " " +
      id +
      "=" +
      v.value +
      " enabled=" +
      checkbox.checked
  );
  let cv = JSON.stringify({
    id: id,
    value: v.value,
    enabled: checkbox.checked,
  });
  sendControlMessage(cv, myr_controls[id].name);
}

function do_button(e) {
  let v = e.target.previousElementSibling;
  let id = html_to_server_id(v.id);
  console.log("set_button " + e + " " + id + "=" + v.value);
  let cv = JSON.stringify({ id: id, value: v.value });
  sendControlMessage(cv, myr_controls[id].name);
}

function do_input(e) {
  let v = e.target;
  console.log("input " + e + " " + v.id + "=" + v.value);
}

function get_element_by_server_id(id) {
  let did = prefix + id;
  let r = document.getElementById(did);
  return r;
}

function html_to_server_id(id) {
  let r = id;
  if (r.startsWith(prefix)) {
    r = r.substr(prefix.length);
  }
  return html_to_value_id(r);
}

function html_to_value_id(id) {
  let r = id;
  if (r.endsWith(auto_postfix)) {
    r = r.substr(0, r.length - auto_postfix.length);
  }
  if (r.endsWith(enabled_postfix)) {
    r = r.substr(0, r.length - enabled_postfix.length);
  }
  return r;
}

function handle_range_unit_change(value) {
  let unit = value == 0 ? / (k?)m$/ : / nm$/;

  if (myr_range_control_id) {
    let e = get_element_by_server_id(myr_range_control_id);
    // Rebuild the select elements from scratch
    let c = myr_controls[myr_range_control_id];

    let validValues = Array();
    let descriptions = {};

    for (const r of c.validValues) {
      if (c.descriptions[r].match(unit)) {
        validValues.push(r);
        descriptions[r] = c.descriptions[r];
      }
    }

    e.innerHTML = "";
    van.add(
      e,
      validValues.map((v) => option({ value: v }, descriptions[v]))
    );
  }
}
