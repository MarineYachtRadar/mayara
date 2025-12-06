export { render_webgpu };

import { RANGE_SCALE, formatRangeValue, is_metric } from "./viewer.js";

class render_webgpu {
  // The constructor gets two canvases, the real drawing one and one for background data
  // such as range circles etc.
  constructor(canvas_dom, canvas_background_dom, drawBackground) {
    this.dom = canvas_dom;
    this.background_dom = canvas_background_dom;
    this.background_ctx = this.background_dom.getContext("2d");
    this.drawBackgroundCallback = drawBackground;

    this.actual_range = 0;
    this.ready = false;
    this.pendingLegend = null;
    this.pendingSpokes = null;

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

    // Create shader module
    this.shaderModule = this.device.createShaderModule({
      code: shaderCode,
    });

    // Create sampler
    this.sampler = this.device.createSampler({
      magFilter: "linear",
      minFilter: "linear",
      addressModeU: "clamp-to-edge",
      addressModeV: "clamp-to-edge",
    });

    // Create uniform buffer for transformation matrix
    this.uniformBuffer = this.device.createBuffer({
      size: 64, // 4x4 matrix = 16 floats = 64 bytes
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    // Create vertex buffer for fullscreen quad
    // TexCoords match WebGL: [0,0], [1,0], [0,1], [1,1]
    const vertices = new Float32Array([
      // Position (x, y), TexCoord (u, v)
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

    this.ready = true;
    this.redrawCanvas();

    // Apply pending calls if they were made before init completed
    if (this.pendingSpokes) {
      this.setSpokes(this.pendingSpokes.spokesPerRevolution, this.pendingSpokes.max_spoke_len);
      this.pendingSpokes = null;
    }
    if (this.pendingLegend) {
      this.setLegend(this.pendingLegend);
      this.pendingLegend = null;
    }
    console.log("WebGPU initialized successfully");
  }

  // This is called as soon as it is clear what the number of spokes and their max length is
  setSpokes(spokesPerRevolution, max_spoke_len) {
    console.log("WebGPU setSpokes:", spokesPerRevolution, max_spoke_len, "ready:", this.ready);

    if (!this.ready) {
      this.pendingSpokes = { spokesPerRevolution, max_spoke_len };
      // Still create CPU buffer for data accumulation
      this.spokesPerRevolution = spokesPerRevolution;
      this.max_spoke_len = max_spoke_len;
      this.data = new Uint8Array(spokesPerRevolution * max_spoke_len);
      return;
    }

    this.spokesPerRevolution = spokesPerRevolution;
    this.max_spoke_len = max_spoke_len;

    // CPU-side buffer for spoke data
    this.data = new Uint8Array(spokesPerRevolution * max_spoke_len);

    // Create polar data texture
    this.polarTexture = this.device.createTexture({
      size: [max_spoke_len, spokesPerRevolution],
      format: "r8unorm",
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
    });

    this.#createPipelineAndBindGroup();
  }

  setRange(range) {
    this.range = range;
    this.redrawCanvas();
  }

  // A new "legend" of what each byte means in terms of suggested color and meaning.
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

    // Create color table texture
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
      this.#createPipelineAndBindGroup();
    }
  }

  #createPipelineAndBindGroup() {
    if (!this.polarTexture || !this.colorTexture) return;

    // Create bind group layout
    const bindGroupLayout = this.device.createBindGroupLayout({
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.FRAGMENT,
          texture: { sampleType: "float" },
        },
        {
          binding: 1,
          visibility: GPUShaderStage.FRAGMENT,
          texture: { sampleType: "float" },
        },
        {
          binding: 2,
          visibility: GPUShaderStage.FRAGMENT,
          sampler: { type: "filtering" },
        },
        {
          binding: 3,
          visibility: GPUShaderStage.VERTEX,
          buffer: { type: "uniform" },
        },
      ],
    });

    // Create pipeline layout
    const pipelineLayout = this.device.createPipelineLayout({
      bindGroupLayouts: [bindGroupLayout],
    });

    // Create render pipeline
    this.pipeline = this.device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: this.shaderModule,
        entryPoint: "vertexMain",
        buffers: [
          {
            arrayStride: 16, // 4 floats * 4 bytes
            attributes: [
              { shaderLocation: 0, offset: 0, format: "float32x2" },  // position
              { shaderLocation: 1, offset: 8, format: "float32x2" },  // texCoord
            ],
          },
        ],
      },
      fragment: {
        module: this.shaderModule,
        entryPoint: "fragmentMain",
        targets: [{ format: this.canvasFormat }],
      },
      primitive: {
        topology: "triangle-strip",
      },
    });

    // Create bind group
    this.bindGroup = this.device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: this.polarTexture.createView() },
        { binding: 1, resource: this.colorTexture.createView() },
        { binding: 2, resource: this.sampler },
        { binding: 3, resource: { buffer: this.uniformBuffer } },
      ],
    });
  }

  // A new spoke has been received.
  drawSpoke(spoke) {
    if (!this.data) return;

    if (this.actual_range != spoke.range) {
      this.actual_range = spoke.range;
      this.redrawCanvas();
    }

    let offset = spoke.angle * this.max_spoke_len;
    this.data.set(spoke.data, offset);
    if (spoke.data.length < this.max_spoke_len) {
      this.data.fill(0, offset + spoke.data.length, offset + this.max_spoke_len);
    }
  }

  // Render accumulated spokes to screen
  render() {
    if (!this.ready || !this.data || !this.pipeline) {
      console.log("WebGPU render skipped - ready:", this.ready, "data:", !!this.data, "pipeline:", !!this.pipeline);
      return;
    }

    // Upload spoke data to GPU
    this.device.queue.writeTexture(
      { texture: this.polarTexture },
      this.data,
      { bytesPerRow: this.max_spoke_len },
      { width: this.max_spoke_len, height: this.spokesPerRevolution }
    );

    // Create command encoder and render pass
    const encoder = this.device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: this.context.getCurrentTexture().createView(),
          clearValue: { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },  // Transparent background
          loadOp: "clear",
          storeOp: "store",
        },
      ],
    });

    pass.setPipeline(this.pipeline);
    pass.setBindGroup(0, this.bindGroup);
    pass.setVertexBuffer(0, this.vertexBuffer);
    pass.draw(4);
    pass.end();

    this.device.queue.submit([encoder.finish()]);
  }

  // Called on initial setup and whenever the canvas size changes.
  redrawCanvas() {
    var parent = this.dom.parentNode,
      styles = getComputedStyle(parent),
      w = parseInt(styles.getPropertyValue("width"), 10),
      h = parseInt(styles.getPropertyValue("height"), 10);

    this.dom.width = w;
    this.dom.height = h;
    this.background_dom.width = w;
    this.background_dom.height = h;

    this.width = this.dom.width;
    this.height = this.dom.height;
    this.center_x = this.width / 2;
    this.center_y = this.height / 2;
    this.beam_length = Math.trunc(
      Math.max(this.center_x, this.center_y) * RANGE_SCALE
    );

    this.drawBackgroundCallback(this, "MAYARA (WebGPU)");

    if (this.ready) {
      this.context.configure({
        device: this.device,
        format: this.canvasFormat,
        alphaMode: "premultiplied",
      });
      this.#setTransformationMatrix();
    }
  }

  #setTransformationMatrix() {
    const range = this.range || this.actual_range || 1500;
    const scale = (1.0 * this.actual_range) / range;
    const angle = Math.PI / 2;

    const scaleX = scale * ((2 * this.beam_length) / this.width);
    const scaleY = scale * ((2 * this.beam_length) / this.height);

    // Combined rotation and scaling matrix (column-major for WebGPU)
    const cos = Math.cos(angle);
    const sin = Math.sin(angle);

    const transformMatrix = new Float32Array([
      cos * scaleX, -sin * scaleX, 0.0, 0.0,
      sin * scaleY,  cos * scaleY, 0.0, 0.0,
      0.0, 0.0, 1.0, 0.0,
      0.0, 0.0, 0.0, 1.0,
    ]);

    this.device.queue.writeBuffer(this.uniformBuffer, 0, transformMatrix);

    this.background_ctx.fillStyle = "lightgreen";
    this.background_ctx.fillText("Beamlength " + this.beam_length, 5, 40);
    this.background_ctx.fillText(
      "Range " + formatRangeValue(is_metric(range), range),
      5,
      60
    );
    this.background_ctx.fillText("Spoke " + this.actual_range, 5, 80);
  }
}

const shaderCode = `
struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) texCoord: vec2<f32>,
}

@group(0) @binding(3) var<uniform> u_transform: mat4x4<f32>;

@vertex
fn vertexMain(
  @location(0) pos: vec2<f32>,
  @location(1) texCoord: vec2<f32>
) -> VertexOutput {
  var output: VertexOutput;
  output.position = u_transform * vec4<f32>(pos, 0.0, 1.0);
  output.texCoord = texCoord;
  return output;
}

@group(0) @binding(0) var polarData: texture_2d<f32>;
@group(0) @binding(1) var colorTable: texture_2d<f32>;
@group(0) @binding(2) var texSampler: sampler;

@fragment
fn fragmentMain(@location(0) texCoord: vec2<f32>) -> @location(0) vec4<f32> {
  // Convert texture coordinates to polar coordinates
  let centered = texCoord - vec2<f32>(0.5, 0.5);
  let r = length(centered) * 2.0;
  let theta = atan2(centered.y, centered.x);

  // Normalize theta to [0, 1] range (matches WebGL shader)
  let normalizedTheta = 1.0 - (theta + 3.14159265) / (2.0 * 3.14159265);

  // Sample the index from polar data texture (always sample, avoid non-uniform control flow)
  let index = textureSample(polarData, texSampler, vec2<f32>(r, normalizedTheta)).r;

  // Look up color from color table
  let color = textureSample(colorTable, texSampler, vec2<f32>(index, 0.0));

  // Use step functions to mask out areas (avoids if statements)
  // Outside radar circle (r > 1.0) or index 0 (no data) -> transparent
  let insideCircle = step(r, 1.0);  // 1.0 if r <= 1.0, else 0.0
  let hasData = step(0.004, index);  // 1.0 if index >= 0.004, else 0.0
  let alpha = insideCircle * hasData * color.a;

  return vec4<f32>(color.rgb, alpha);
}
`;
