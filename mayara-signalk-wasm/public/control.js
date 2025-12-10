export { loadRadar, registerRadarCallback, registerControlCallback };

import van from "./van-1.5.2.js";
import { fetchRadars, detectMode, sendControlCommand } from "./api.js";

const { div, label, input, button, select, option } = van.tags;

const prefix = "myr_";
const auto_postfix = "_auto";
const enabled_postfix = "_enabled";

const RANGE_UNIT_SELECT_ID = 999;

var myr_radar;
var myr_controls;
var myr_range_control_id;
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

  for (const [k, v] of Object.entries(myr_controls)) {
    if (v["isReadOnly"]) {
      if (k == 0) {
        van.add(c, div({ class: "myr_control myr_error" }, "REPLAY MODE"));
      }
      van.add(c, ReadOnlyValue(k, v.name));
    } else if (v["dataType"] == "button") {
      van.add(c, ButtonValue(k, v.name));
    } else if (v["dataType"] == "string") {
      van.add(c, StringValue(k, v.name));
      van.add(get_element_by_server_id(k).parentNode, SetButton());
    } else if ("validValues" in v && "descriptions" in v) {
      if (v.name == "Range") {
        add_range_unit_select(c, v["descriptions"]);
      }
      van.add(c, SelectValue(k, v.name, v["validValues"], v["descriptions"]));
    } else if ("maxValue" in v && v.maxValue <= 100) {
      van.add(
        c,
        RangeValue(k, v.name, v.minValue, v.maxValue, 0, "descriptions" in v)
      );
    } else {
      van.add(c, NumericValue(k, v.name));
    }
    if (v["hasAuto"]) {
      van.add(get_element_by_server_id(k).parentNode, AutoButton(k));
    }
    if (v["hasEnabled"] && !v["isReadOnly"]) {
      van.add(get_element_by_server_id(k).parentNode, EnabledButton(k));
    }
  }
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
