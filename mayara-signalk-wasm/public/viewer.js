"use strict";

export { RANGE_SCALE, formatRangeValue, is_metric };

import {
  loadRadar,
  registerRadarCallback,
  registerControlCallback,
} from "./control.js";
import "./protobuf/protobuf.min.js";

const prefix = "myr_";

import { render_2d } from "./render_2d.js";
import { render_webgl } from "./render_webgl.js";
import { render_webgl_alt } from "./render_webgl_alt.js";
import { render_webgpu } from "./render_webgpu.js";

var webSocket;
var RadarMessage;
var renderer;
var noTransmitAngles = Array();

function divides_near(a, b) {
  let remainder = a % b;
  let r = remainder <= 1.0 || remainder >= b - 1;
  console.log(
    "divides_near: " + a + " % " + b + " = " + remainder + " -> " + r
  );
  return r;
}

function is_metric(v) {
  if (v <= 100) {
    return divides_near(v, 25);
  } else if (v <= 750) {
    return divides_near(v, 50);
  }
  return divides_near(v, 500);
}

const NAUTICAL_MILE = 1852.0;

function formatRangeValue(metric, v) {
  if (metric) {
    // Metric
    v = Math.round(v);
    if (v >= 1000) {
      return v / 1000 + " km";
    } else {
      return v + " m";
    }
  } else {
    if (v >= NAUTICAL_MILE - 1) {
      if (divides_near(v, NAUTICAL_MILE)) {
        return Math.floor((v + 1) / NAUTICAL_MILE) + " nm";
      } else {
        return v / NAUTICAL_MILE + " nm";
      }
    } else if (divides_near(v, NAUTICAL_MILE / 2)) {
      return Math.floor((v + 1) / (NAUTICAL_MILE / 2)) + "/2 nm";
    } else if (divides_near(v, NAUTICAL_MILE / 4)) {
      return Math.floor((v + 1) / (NAUTICAL_MILE / 4)) + "/4 nm";
    } else if (divides_near(v, NAUTICAL_MILE / 8)) {
      return Math.floor((v + 1) / (NAUTICAL_MILE / 8)) + "/8 nm";
    } else if (divides_near(v, NAUTICAL_MILE / 16)) {
      return Math.floor((v + 1) / (NAUTICAL_MILE / 16)) + "/16 nm";
    } else if (divides_near(v, NAUTICAL_MILE / 32)) {
      return Math.floor((v + 1) / (NAUTICAL_MILE / 32)) + "/32 nm";
    } else if (divides_near(v, NAUTICAL_MILE / 64)) {
      return Math.floor((v + 1) / (NAUTICAL_MILE / 64)) + "/64 nm";
    } else if (divides_near(v, NAUTICAL_MILE / 128)) {
      return Math.floor((v + 1) / (NAUTICAL_MILE / 128)) + "/128 nm";
    } else {
      return v / NAUTICAL_MILE + " nm";
    }
  }
}

const RANGE_SCALE = 0.9; // Factor by which we fill the (w,h) canvas with the outer radar range ring

registerRadarCallback(radarLoaded);
registerControlCallback(controlUpdate);

window.onload = function () {
  const urlParams = new URLSearchParams(window.location.search);
  const id = urlParams.get("id");
  const draw = urlParams.get("draw");

  protobuf.load("./proto/RadarMessage.proto", function (err, root) {
    if (err) throw err;

    RadarMessage = root.lookupType(".RadarMessage");
  });

  try {
    if (draw == "2d") {
      renderer = new render_2d(
        document.getElementById("myr_canvas"),
        document.getElementById("myr_canvas_background"),
        drawBackground
      );
    } else if (draw == "alt") {
      renderer = new render_webgl_alt(
        document.getElementById("myr_canvas_webgl"),
        document.getElementById("myr_canvas_background"),
        drawBackground
      );
    } else if (draw == "webgpu") {
      renderer = new render_webgpu(
        document.getElementById("myr_canvas_webgpu"),
        document.getElementById("myr_canvas_background"),
        drawBackground
      );
    } else {
      renderer = new render_webgl(
        document.getElementById("myr_canvas_webgl"),
        document.getElementById("myr_canvas_background"),
        drawBackground
      );
    }
  } catch (e) {
    console.log(e);
    console.log("Falling back on 2d context");
    renderer = new render_2d(
      document.getElementById("myr_canvas"),
      document.getElementById("myr_canvas_background"),
      drawBackground
    );
  }

  loadRadar(id);

  window.onresize = function () {
    renderer.redrawCanvas();
  };
};

function restart(id) {
  setTimeout(loadRadar, 15000, id);
}

function radarLoaded(r) {
  let maxSpokeLen = r.maxSpokeLen;
  let spokesPerRevolution = r.spokesPerRevolution;
  let prev_angle = -1;

  if (r === undefined || r.controls === undefined) {
    return;
  }
  renderer.setLegend(expandLegend(r.legend));
  renderer.setSpokes(spokesPerRevolution, maxSpokeLen);

  // Use provided streamUrl or construct SignalK stream URL
  // Check for undefined, null, empty, or literal string "undefined"
  let streamUrl = r.streamUrl;
  if (!streamUrl || streamUrl === "undefined" || streamUrl === "null") {
    const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    streamUrl = `${wsProtocol}//${window.location.host}/signalk/v2/api/vessels/self/radars/${r.id}/stream`;
  }
  console.log("Connecting to radar stream:", streamUrl);
  webSocket = new WebSocket(streamUrl);
  webSocket.binaryType = "arraybuffer";

  webSocket.onopen = (e) => {
    console.log("websocket open: " + JSON.stringify(e));
  };
  webSocket.onclose = (e) => {
    console.log("websocket close: code=" + e.code + ", reason=" + e.reason + ", wasClean=" + e.wasClean);
    restart(r.id);
  };
  webSocket.onerror = (e) => {
    console.log("websocket error:", e);
  };
  webSocket.onmessage = (e) => {
    if (RadarMessage) {
      let buf = e.data;
      let bytes = new Uint8Array(buf);
      var message = RadarMessage.decode(bytes);
      if (message.spokes) {
        for (let i = 0; i < message.spokes.length; i++) {
          let spoke = message.spokes[i];

          // The number of spokes actually sent can be lower than the stated angles,
          // fill out the spokes between prev_angle and spoke.angle by generating a blankish spoke.
          if (
            prev_angle > -1 &&
            (prev_angle + 1) % spokesPerRevolution != spoke.angle
          ) {
            prev_angle = (prev_angle + 1) % spokesPerRevolution;
            let empty_spoke = {
              angle: prev_angle,
              bearing: spoke.bearing,
              range: spoke.range,
              data: new Uint8Array(maxSpokeLen).fill(1),
            };
            if (prev_angle > spoke.angle) {
              for (
                let angle = prev_angle + 1;
                angle < spokesPerRevolution;
                angle++
              ) {
                empty_spoke.angle = angle;
                renderer.drawSpoke(empty_spoke);
              }
              prev_angle = -1; // Reset to -1 so we can start at 0 again
            }
            if (prev_angle < spoke.angle) {
              for (let angle = prev_angle + 1; angle < spoke.angle; angle++) {
                empty_spoke.angle = angle;
                renderer.drawSpoke(empty_spoke);
              }
            }
          }
          renderer.drawSpoke(spoke);
          prev_angle = spoke.angle;
        }
        renderer.render();
      }
    }
  };
}

function expandLegend(legend) {
  let a = Array();
  for (let i = 0; i < Object.keys(legend).length; i++) {
    let color = legend[i].color;
    a.push(hexToRGBA(color));
  }
  a[0][3] = 255;

  return a;
}

function hexToRGBA(hex) {
  let a = Array();
  for (let i = 1; i < hex.length; i += 2) {
    a.push(parseInt(hex.slice(i, i + 2), 16));
  }
  while (a.length < 3) {
    a.push(0);
  }
  while (a.length < 4) {
    a.push(255);
  }

  return a;
}

function controlUpdate(control, controlValue) {
  if (control.name == "Range") {
    let range = parseFloat(controlValue.value);
    renderer.setRange(range);
  }
  if (control.name.startsWith("No Transmit")) {
    let value = parseFloat(controlValue.value);
    let idx = extractNoTxZone(control.name);
    let start_or_end = extractStartOrEnd(control.name);
    if (controlValue.enabled) {
      noTransmitAngles[idx][start_or_end] = value;
    } else {
      noTransmitAngles[idx] = null;
    }
  }
}

function extractNoTxZone(name) {
  const re = /(\d+)/;
  let match = name.match(re);
  if (match) {
    return parseInt(match[1]);
  }
  return 0;
}

function extractStartOrEnd(name) {
  return name.includes("start") ? 0 : 1;
}

function drawBackground(obj, txt) {
  obj.background_ctx.setTransform(1, 0, 0, 1, 0, 0);
  obj.background_ctx.clearRect(0, 0, obj.width, obj.height);

  // Use actual_range as fallback if range not set from controls
  const range = obj.range || obj.actual_range;

  obj.background_ctx.strokeStyle = "#00ff00";
  obj.background_ctx.fillStyle = "#00ff00";
  obj.background_ctx.font = "bold 16px/1 Verdana, Geneva, sans-serif";
  for (let i = 0; i <= 4; i++) {
    obj.background_ctx.beginPath();
    obj.background_ctx.arc(
      obj.center_x,
      obj.center_y,
      (i * obj.beam_length) / 4,
      0,
      2 * Math.PI
    );
    obj.background_ctx.stroke();
    if (i > 0 && range) {
      let text = formatRangeValue(is_metric(range), (range * i) / 4);
      obj.background_ctx.fillText(
        text,
        obj.center_x + (i * obj.beam_length * 1.41) / 8,
        obj.center_y + (i * obj.beam_length * -1.41) / 8
      );
    }
  }

  obj.background_ctx.fillStyle = "lightgrey";

  if (typeof noTransmitAngles == "array") {
    noTransmitAngles.forEach((e) => {
      if (e && e[0]) {
        obj.background_ctx.beginPath();
        obj.background_ctx.arc(
          obj.center_x,
          obj.center_y,
          obj.beam_length * 2,
          (2 * Math.PI * e[0]) / obj.spokesPerRevolution,
          (2 * Math.PI * e[1]) / obj.spokesPerRevolution
        );
        obj.background_ctx.fill();
      }
    });
  }
  obj.background_ctx.fillStyle = "lightblue";
  this.background_ctx.fillText(txt, 5, 20);
}
