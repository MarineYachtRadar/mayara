export { render_webgpu };

import { RANGE_SCALE, formatRangeValue, is_metric, getHeadingMode, getTrueHeading } from "./viewer.js";

class render_webgpu {
  constructor(canvas_dom, canvas_background_dom, drawBackground) {
    this.dom = canvas_dom;
    this.background_dom = canvas_background_dom;
    this.background_ctx = this.background_dom.getContext("2d");
    // Overlay canvas for range rings (on top of radar)
    this.overlay_dom = document.getElementById("myr_canvas_overlay");
    this.overlay_ctx = this.overlay_dom ? this.overlay_dom.getContext("2d") : null;
    this.drawBackgroundCallback = drawBackground;

    this.actual_range = 0;
    this.ready = false;
    this.pendingLegend = null;
    this.pendingSpokes = null;

    // Rotation tracking for neighbor enhancement
    this.rotationCount = 0;
    this.lastSpokeAngle = -1;
    this.fillRotations = 4; // Number of rotations to use neighbor enhancement

    // Heading rotation for North Up mode (in radians)
    this.headingRotation = 0;

    // Start async initialization
    this.initPromise = this.#initWebGPU();
  }

  async #initWebGPU() {
    if (!navigator.gpu) {
      throw new Error("WebGPU not supported");
    }

    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) {
      throw new Error("No WebGPU adapter found");
    }

    this.device = await adapter.requestDevice();
    this.context = this.dom.getContext("webgpu");

    this.canvasFormat = navigator.gpu.getPreferredCanvasFormat();
    this.context.configure({
      device: this.device,
      format: this.canvasFormat,
      alphaMode: "premultiplied",
    });

    // Create sampler for polar data (linear for smooth display like TZ Pro)
    this.sampler = this.device.createSampler({
      magFilter: "linear",
      minFilter: "linear",
      addressModeU: "clamp-to-edge",
      addressModeV: "repeat",  // Wrap around for angles
    });

    // Create uniform buffer for parameters
    this.uniformBuffer = this.device.createBuffer({
      size: 32,  // scaleX, scaleY, spokesPerRev, maxSpokeLen + padding
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    // Create vertex buffer for fullscreen quad
    const vertices = new Float32Array([
      -1.0, -1.0, 0.0, 0.0,
       1.0, -1.0, 1.0, 0.0,
      -1.0,  1.0, 0.0, 1.0,
       1.0,  1.0, 1.0, 1.0,
    ]);

    this.vertexBuffer = this.device.createBuffer({
      size: vertices.byteLength,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(this.vertexBuffer, 0, vertices);

    // Create render pipeline
    await this.#createRenderPipeline();

    this.ready = true;
    this.redrawCanvas();

    if (this.pendingSpokes) {
      this.setSpokes(this.pendingSpokes.spokesPerRevolution, this.pendingSpokes.max_spoke_len);
      this.pendingSpokes = null;
    }
    if (this.pendingLegend) {
      this.setLegend(this.pendingLegend);
      this.pendingLegend = null;
    }
    console.log("WebGPU initialized (direct polar rendering)");
  }

  async #createRenderPipeline() {
    const shaderModule = this.device.createShaderModule({
      code: shaderCode,
    });

    this.bindGroupLayout = this.device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } }, // polar data
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } }, // color table
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        { binding: 3, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
      ],
    });

    this.renderPipeline = this.device.createRenderPipeline({
      layout: this.device.createPipelineLayout({ bindGroupLayouts: [this.bindGroupLayout] }),
      vertex: {
        module: shaderModule,
        entryPoint: "vertexMain",
        buffers: [{
          arrayStride: 16,
          attributes: [
            { shaderLocation: 0, offset: 0, format: "float32x2" },
            { shaderLocation: 1, offset: 8, format: "float32x2" },
          ],
        }],
      },
      fragment: {
        module: shaderModule,
        entryPoint: "fragmentMain",
        targets: [{ format: this.canvasFormat }],
      },
      primitive: { topology: "triangle-strip" },
    });
  }

  setSpokes(spokesPerRevolution, max_spoke_len) {
    console.log("WebGPU setSpokes:", spokesPerRevolution, max_spoke_len, "ready:", this.ready);

    if (!this.ready) {
      this.pendingSpokes = { spokesPerRevolution, max_spoke_len };
      this.spokesPerRevolution = spokesPerRevolution;
      this.max_spoke_len = max_spoke_len;
      this.data = new Uint8Array(spokesPerRevolution * max_spoke_len);
      return;
    }

    this.spokesPerRevolution = spokesPerRevolution;
    this.max_spoke_len = max_spoke_len;
    this.data = new Uint8Array(spokesPerRevolution * max_spoke_len);

    // Create polar data texture (width = range samples, height = angles)
    this.polarTexture = this.device.createTexture({
      size: [max_spoke_len, spokesPerRevolution],
      format: "r8unorm",
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
    });

    this.#createBindGroup();
  }

  setRange(range) {
    this.range = range;
    // Clear spoke data when range changes - old data is no longer valid
    if (this.data) {
      this.data.fill(0);
    }
    this.redrawCanvas();
  }

  setHeadingRotation(radians) {
    this.headingRotation = radians;
    if (this.ready) {
      this.#updateUniforms();
    }
  }

  setLegend(l) {
    console.log("WebGPU setLegend, ready:", this.ready);
    if (!this.ready) {
      this.pendingLegend = l;
      return;
    }

    const colorTableData = new Uint8Array(256 * 4);
    for (let i = 0; i < l.length; i++) {
      colorTableData[i * 4] = l[i][0];
      colorTableData[i * 4 + 1] = l[i][1];
      colorTableData[i * 4 + 2] = l[i][2];
      colorTableData[i * 4 + 3] = l[i][3];
    }

    this.colorTexture = this.device.createTexture({
      size: [256, 1],
      format: "rgba8unorm",
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
    });

    this.device.queue.writeTexture(
      { texture: this.colorTexture },
      colorTableData,
      { bytesPerRow: 256 * 4 },
      { width: 256, height: 1 }
    );

    if (this.polarTexture) {
      this.#createBindGroup();
    }
  }

  #createBindGroup() {
    if (!this.polarTexture || !this.colorTexture) return;

    this.bindGroup = this.device.createBindGroup({
      layout: this.bindGroupLayout,
      entries: [
        { binding: 0, resource: this.polarTexture.createView() },
        { binding: 1, resource: this.colorTexture.createView() },
        { binding: 2, resource: this.sampler },
        { binding: 3, resource: { buffer: this.uniformBuffer } },
      ],
    });
  }

  drawSpoke(spoke) {
    if (!this.data) return;

    if (this.actual_range != spoke.range) {
      this.actual_range = spoke.range;
      // Clear spoke data when range changes - old data is at wrong scale
      this.data.fill(0);
      // Reset rotation counter on range change
      this.rotationCount = 0;
      this.lastSpokeAngle = -1;
      this.redrawCanvas();
    }

    // Bounds check - log bad angles
    if (spoke.angle >= this.spokesPerRevolution) {
      console.error(`Bad spoke angle: ${spoke.angle} >= ${this.spokesPerRevolution}`);
      return;
    }

    // Track rotations: detect when we wrap around from high angle to low angle
    if (this.lastSpokeAngle >= 0 && spoke.angle < this.lastSpokeAngle - this.spokesPerRevolution / 2) {
      this.rotationCount++;
    }
    this.lastSpokeAngle = spoke.angle;

    let offset = spoke.angle * this.max_spoke_len;

    // Check if data fits in buffer
    if (offset + spoke.data.length > this.data.length) {
      console.error(`Buffer overflow: offset=${offset}, data.len=${spoke.data.length}, buf.len=${this.data.length}, angle=${spoke.angle}, maxSpokeLen=${this.max_spoke_len}, spokes=${this.spokesPerRevolution}`);
      return;
    }

    const spokeLen = spoke.data.length;
    const maxLen = this.max_spoke_len;

    // Only use neighbor enhancement during first few rotations to fill display quickly
    if (this.rotationCount < this.fillRotations) {
      // Write spoke data with neighbor enhancement
      // Strong signals spread wider, weak signals spread less
      const spokes = this.spokesPerRevolution;

      for (let i = 0; i < spokeLen; i++) {
        const val = spoke.data[i];
        // Write current spoke at full value
        this.data[offset + i] = val;

        if (val > 1) {
          // Strong signals (>60): spread wide (±6 spokes) with higher intensity
          // Medium signals (25-60): spread medium (±4 spokes)
          // Weak signals (<25): spread narrow (±2 spokes) with lower intensity
          let spreadWidth, blendFactors;

          if (val > 60) {
            // Strong signal - spread wide and strong
            spreadWidth = 6;
            blendFactors = [0.95, 0.88, 0.78, 0.65, 0.50, 0.35];
          } else if (val > 25) {
            // Medium signal - normal spread
            spreadWidth = 4;
            blendFactors = [0.85, 0.65, 0.45, 0.25];
          } else {
            // Weak signal - narrow spread, lower intensity
            spreadWidth = 2;
            blendFactors = [0.6, 0.3];
          }

          for (let d = 1; d <= spreadWidth; d++) {
            const prev = (spoke.angle + spokes - d) % spokes;
            const next = (spoke.angle + d) % spokes;
            const prevOffset = prev * maxLen;
            const nextOffset = next * maxLen;
            const blendVal = Math.floor(val * blendFactors[d - 1]);

            if (this.data[prevOffset + i] < blendVal) {
              this.data[prevOffset + i] = blendVal;
            }
            if (this.data[nextOffset + i] < blendVal) {
              this.data[nextOffset + i] = blendVal;
            }
          }
        }
      }
    } else {
      // RUN mode: smart filtering
      // - Strong signals with neighbor support get amplified aggressively (wide check ±4)
      // - Isolated weak signals (scatter) get killed
      const spokes = this.spokesPerRevolution;

      // Wide neighbor check for strong signals: ±4 spokes
      const prev1Offset = ((spoke.angle + spokes - 1) % spokes) * maxLen;
      const prev2Offset = ((spoke.angle + spokes - 2) % spokes) * maxLen;
      const prev3Offset = ((spoke.angle + spokes - 3) % spokes) * maxLen;
      const prev4Offset = ((spoke.angle + spokes - 4) % spokes) * maxLen;
      const next1Offset = ((spoke.angle + 1) % spokes) * maxLen;
      const next2Offset = ((spoke.angle + 2) % spokes) * maxLen;
      const next3Offset = ((spoke.angle + 3) % spokes) * maxLen;
      const next4Offset = ((spoke.angle + 4) % spokes) * maxLen;

      for (let i = 0; i < spokeLen; i++) {
        const val = spoke.data[i];

        // Check neighbor support (from previous rotation's data still in buffer)
        const prev1 = this.data[prev1Offset + i];
        const prev2 = this.data[prev2Offset + i];
        const prev3 = this.data[prev3Offset + i];
        const prev4 = this.data[prev4Offset + i];
        const next1 = this.data[next1Offset + i];
        const next2 = this.data[next2Offset + i];
        const next3 = this.data[next3Offset + i];
        const next4 = this.data[next4Offset + i];

        // For strong signals: use wide sum (±4)
        const wideSum = prev1 + prev2 + prev3 + prev4 + next1 + next2 + next3 + next4;
        const wideMax = Math.max(prev1, prev2, prev3, prev4, next1, next2, next3, next4);
        // For weak signals: use narrow sum (±2)
        const narrowSum = prev1 + prev2 + next1 + next2;
        const narrowMax = Math.max(prev1, prev2, next1, next2);

        let outputVal;

        if (val > 60) {
          // Strong signal: use wide neighbor check (±4)
          if (wideSum > 200) {
            // Solid mass - boost hard and spread to neighbors
            outputVal = Math.min(255, Math.floor(val * 1.35));
            // Boost immediate neighbors to fill gaps
            if (prev1 > 25) this.data[prev1Offset + i] = Math.min(255, Math.floor(prev1 * 1.15));
            if (next1 > 25) this.data[next1Offset + i] = Math.min(255, Math.floor(next1 * 1.15));
            if (prev2 > 25) this.data[prev2Offset + i] = Math.min(255, Math.floor(prev2 * 1.1));
            if (next2 > 25) this.data[next2Offset + i] = Math.min(255, Math.floor(next2 * 1.1));
          } else if (wideMax > 50) {
            // Some support - moderate boost
            outputVal = Math.min(255, Math.floor(val * 1.2));
          } else {
            // Strong but isolated - suspicious, reduce
            outputVal = Math.floor(val * 0.8);
          }
        } else if (val > 25) {
          // Medium signal: needs good neighbor support
          if (narrowSum > 80) {
            // Good support - boost it
            outputVal = Math.min(255, Math.floor(val * 1.2));
          } else if (narrowMax > 40) {
            // Some support - keep
            outputVal = val;
          } else {
            // Isolated medium - likely scatter, punish hard
            outputVal = Math.floor(val * 0.4);
          }
        } else if (val > 1) {
          // Weak signal: kill it unless very well supported
          if (narrowSum > 100) {
            // Strong neighbors - this might be edge of real target
            outputVal = val;
          } else if (narrowMax > 60) {
            // Next to something strong - keep faint
            outputVal = Math.floor(val * 0.5);
          } else {
            // Isolated weak signal - kill it
            outputVal = 0;
          }
        } else {
          outputVal = val;
        }

        this.data[offset + i] = outputVal;
      }
    }

    // Clear remainder of spoke if data is shorter than max
    if (spokeLen < maxLen) {
      this.data.fill(0, offset + spokeLen, offset + maxLen);
    }
  }

  render() {
    if (!this.ready || !this.data || !this.bindGroup) {
      return;
    }

    // Upload spoke data to GPU
    this.device.queue.writeTexture(
      { texture: this.polarTexture },
      this.data,
      { bytesPerRow: this.max_spoke_len },
      { width: this.max_spoke_len, height: this.spokesPerRevolution }
    );

    const encoder = this.device.createCommandEncoder();

    const renderPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: this.context.getCurrentTexture().createView(),
        clearValue: { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
        loadOp: "clear",
        storeOp: "store",
      }],
    });
    renderPass.setPipeline(this.renderPipeline);
    renderPass.setBindGroup(0, this.bindGroup);
    renderPass.setVertexBuffer(0, this.vertexBuffer);
    renderPass.draw(4);
    renderPass.end();

    this.device.queue.submit([encoder.finish()]);
  }

  redrawCanvas() {
    var parent = this.dom.parentNode,
      styles = getComputedStyle(parent),
      w = parseInt(styles.getPropertyValue("width"), 10),
      h = parseInt(styles.getPropertyValue("height"), 10);

    this.dom.width = w;
    this.dom.height = h;
    this.background_dom.width = w;
    this.background_dom.height = h;
    if (this.overlay_dom) {
      this.overlay_dom.width = w;
      this.overlay_dom.height = h;
    }

    this.width = this.dom.width;
    this.height = this.dom.height;
    this.center_x = this.width / 2;
    this.center_y = this.height / 2;
    this.beam_length = Math.trunc(
      Math.max(this.center_x, this.center_y) * RANGE_SCALE
    );

    this.drawBackgroundCallback(this, "MAYARA (WebGPU)");
    this.#drawOverlay();

    if (this.ready) {
      this.context.configure({
        device: this.device,
        format: this.canvasFormat,
        alphaMode: "premultiplied",
      });
      this.#updateUniforms();
    }
  }

  #drawOverlay() {
    if (!this.overlay_ctx) return;

    const ctx = this.overlay_ctx;
    const range = this.range || this.actual_range;

    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.width, this.height);

    // Draw range rings in bright green on top of radar
    ctx.strokeStyle = "#00ff00";
    ctx.lineWidth = 1.5;
    ctx.fillStyle = "#00ff00";
    ctx.font = "bold 14px/1 Verdana, Geneva, sans-serif";

    for (let i = 1; i <= 4; i++) {
      const radius = (i * this.beam_length) / 4;
      ctx.beginPath();
      ctx.arc(this.center_x, this.center_y, radius, 0, 2 * Math.PI);
      ctx.stroke();

      // Draw range labels
      if (range) {
        const text = formatRangeValue(is_metric(range), (range * i) / 4);
        // Position labels at 45 degrees (upper right)
        const labelX = this.center_x + (radius * 0.707);
        const labelY = this.center_y - (radius * 0.707);
        ctx.fillText(text, labelX + 5, labelY - 5);
      }
    }

    // Draw degree markers (compass rose) around the 3rd range ring
    const degreeRingRadius = (3 * this.beam_length) / 4;
    const tickLength = 8;
    const majorTickLength = 12;
    ctx.font = "bold 12px/1 Verdana, Geneva, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    // Get heading mode and true heading for compass rose rotation
    const headingMode = getHeadingMode();
    const trueHeadingRad = getTrueHeading();
    const trueHeadingDeg = (trueHeadingRad * 180) / Math.PI;

    // In Heading Up mode: compass rose rotates so heading is at top
    // In North Up mode: compass rose stays fixed with 0 (N) at top
    const roseRotationDeg = headingMode === "headingUp" ? -trueHeadingDeg : 0;

    for (let deg = 0; deg < 360; deg += 10) {
      // Apply compass rose rotation
      const displayDeg = deg + roseRotationDeg;

      // Radar convention: 0° = top, angles increase clockwise
      // Canvas: 0 radians = right (3 o'clock), increases counter-clockwise
      // So we need: canvasAngle = -displayDeg + 90 (in degrees), or (90 - displayDeg) * PI/180
      const radians = ((90 - displayDeg) * Math.PI) / 180;

      const cos = Math.cos(radians);
      const sin = Math.sin(radians);

      // Determine tick length (longer for cardinal directions)
      const isMajor = deg % 30 === 0;
      const tick = isMajor ? majorTickLength : tickLength;

      // Inner and outer points of tick mark
      const innerRadius = degreeRingRadius - tick / 2;
      const outerRadius = degreeRingRadius + tick / 2;

      const x1 = this.center_x + innerRadius * cos;
      const y1 = this.center_y - innerRadius * sin;
      const x2 = this.center_x + outerRadius * cos;
      const y2 = this.center_y - outerRadius * sin;

      ctx.beginPath();
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.stroke();

      // Draw degree labels at major ticks (every 30°)
      if (isMajor) {
        const labelRadius = degreeRingRadius + majorTickLength + 10;
        const labelX = this.center_x + labelRadius * cos;
        const labelY = this.center_y - labelRadius * sin;
        ctx.fillText(deg.toString(), labelX, labelY);
      }
    }

    // Draw North indicator (N) at 0° position
    const northDeg = roseRotationDeg; // Where 0° (North) appears on screen
    const northRadians = ((90 - northDeg) * Math.PI) / 180;
    const northRadius = degreeRingRadius + majorTickLength + 25;
    const northX = this.center_x + northRadius * Math.cos(northRadians);
    const northY = this.center_y - northRadius * Math.sin(northRadians);
    ctx.font = "bold 14px/1 Verdana, Geneva, sans-serif";
    ctx.fillText("N", northX, northY);
  }

  #updateUniforms() {
    const range = this.range || this.actual_range || 1500;
    const scale = (1.0 * this.actual_range) / range;

    const scaleX = scale * ((2 * this.beam_length) / this.width);
    const scaleY = scale * ((2 * this.beam_length) / this.height);

    // Pack uniforms: scaleX, scaleY, spokesPerRev, maxSpokeLen, headingRotation
    const uniforms = new Float32Array([
      scaleX, scaleY,
      this.spokesPerRevolution || 2048,
      this.max_spoke_len || 512,
      this.headingRotation || 0,  // Heading rotation in radians (for North Up mode)
      0, 0, 0  // padding to 32 bytes
    ]);

    this.device.queue.writeBuffer(this.uniformBuffer, 0, uniforms);

    this.background_ctx.fillStyle = "lightgreen";
    this.background_ctx.fillText("Beam length: " + this.beam_length + " px", 5, 40);
    this.background_ctx.fillText("Display range: " + formatRangeValue(is_metric(range), range), 5, 60);
    this.background_ctx.fillText("Radar range: " + formatRangeValue(is_metric(this.actual_range), this.actual_range), 5, 80);
    this.background_ctx.fillText("Spoke length: " + (this.max_spoke_len || 0) + " px", 5, 100);
  }
}

// Direct polar-to-cartesian shader with color lookup
// Radar convention: angle 0 = bow (up), angles increase CLOCKWISE
// So angle spokesPerRev/4 = starboard (right), spokesPerRev/2 = stern (down)
const shaderCode = `
struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) texCoord: vec2<f32>,
}

struct Uniforms {
  scaleX: f32,
  scaleY: f32,
  spokesPerRev: f32,
  maxSpokeLen: f32,
  headingRotation: f32,  // Rotation in radians for North Up mode
}

@group(0) @binding(3) var<uniform> uniforms: Uniforms;

@vertex
fn vertexMain(@location(0) pos: vec2<f32>, @location(1) texCoord: vec2<f32>) -> VertexOutput {
  var output: VertexOutput;
  // Apply scaling
  let scaledPos = vec2<f32>(pos.x * uniforms.scaleX, pos.y * uniforms.scaleY);
  output.position = vec4<f32>(scaledPos, 0.0, 1.0);
  output.texCoord = texCoord;
  return output;
}

@group(0) @binding(0) var polarData: texture_2d<f32>;
@group(0) @binding(1) var colorTable: texture_2d<f32>;
@group(0) @binding(2) var texSampler: sampler;

const PI: f32 = 3.14159265359;
const TWO_PI: f32 = 6.28318530718;

@fragment
fn fragmentMain(@location(0) texCoord: vec2<f32>) -> @location(0) vec4<f32> {
  // Convert cartesian (texCoord) to polar for sampling radar data
  // texCoord is [0,1]x[0,1], center at (0.5, 0.5)
  //
  // IMPORTANT: In our vertex setup, texCoord.y=0 is BOTTOM, texCoord.y=1 is TOP
  // (WebGPU clip space has Y pointing up)
  // So centered.y is POSITIVE at TOP of screen, NEGATIVE at BOTTOM
  let centered = texCoord - vec2<f32>(0.5, 0.5);

  // Calculate radius (0 at center, 1 at edge of unit circle)
  let r = length(centered) * 2.0;

  // Calculate angle from center for clockwise rotation from top (bow)
  //
  // Our coordinate system (after centering):
  // - Top of screen (bow):      centered = (0, +0.5)
  // - Right of screen (stbd):   centered = (+0.5, 0)
  // - Bottom of screen (stern): centered = (0, -0.5)
  // - Left of screen (port):    centered = (-0.5, 0)
  //
  // Radar convention (from protobuf):
  // - angle 0 = bow (top on screen)
  // - angle increases clockwise: bow -> starboard -> stern -> port -> bow
  //
  // Use atan2(x, y) to get clockwise angle from top:
  // - Top:    (0, 0.5)   -> atan2(0, 0.5) = 0
  // - Right:  (0.5, 0)   -> atan2(0.5, 0) = PI/2
  // - Bottom: (0, -0.5)  -> atan2(0, -0.5) = PI
  // - Left:   (-0.5, 0)  -> atan2(-0.5, 0) = -PI/2 -> normalized to 3PI/2
  var theta = atan2(centered.x, centered.y);

  // Apply heading rotation for North Up mode
  // In North Up: we rotate the radar image by -heading, so we add heading to theta
  // This samples the spoke data at (theta + heading), effectively rotating the display
  theta = theta - uniforms.headingRotation;

  if (theta < 0.0) {
    theta = theta + TWO_PI;
  }
  if (theta >= TWO_PI) {
    theta = theta - TWO_PI;
  }

  // Normalize to [0, 1] for texture V coordinate
  let normalizedTheta = theta / TWO_PI;

  // Sample polar data (always sample, mask later to avoid non-uniform control flow)
  // U = radius [0,1], V = angle [0,1] where 0=bow, 0.25=starboard, 0.5=stern, 0.75=port
  let radarValue = textureSample(polarData, texSampler, vec2<f32>(r, normalizedTheta)).r;

  // Look up color from table
  let color = textureSample(colorTable, texSampler, vec2<f32>(radarValue, 0.0));

  // Mask pixels outside the radar circle (use step instead of if)
  let insideCircle = step(r, 1.0);

  // Use alpha from color table, but make background transparent
  let hasData = step(0.004, radarValue);  // ~1/255 threshold
  let alpha = hasData * color.a * insideCircle;

  return vec4<f32>(color.rgb * insideCircle, alpha);
}
`;
