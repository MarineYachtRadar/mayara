/**
 * WebGPU Spoke Line Renderer
 *
 * Renders raw radar spokes as line segments, similar to signalk-radar's approach.
 * Each spoke is drawn as lines connecting the current spoke angle to the next,
 * with colors looked up from a legend table.
 *
 * This provides a "Furuno chartplotter" look by directly visualizing the raw
 * radar data without blob detection or polygon extraction.
 */

export class SpokeRenderer {
  /**
   * @param {GPUDevice} device - WebGPU device
   * @param {GPUCanvasContext} context - Canvas context
   * @param {string} canvasFormat - Preferred canvas format
   * @param {number} spokesPerRevolution - Number of spokes in one rotation
   * @param {number} maxSpokeLen - Maximum spoke length (range bins)
   */
  constructor(device, context, canvasFormat, spokesPerRevolution, maxSpokeLen) {
    this.device = device;
    this.context = context;
    this.canvasFormat = canvasFormat;
    this.spokesPerRevolution = spokesPerRevolution;
    this.maxSpokeLen = maxSpokeLen;

    // Maximum vertices we can render - limited to stay within WebGPU buffer limits
    // Default max buffer size is 256MB, but we'll use a much smaller fixed size
    // to be safe and performant. 2M vertices = 2M * 6 floats * 4 bytes = 48MB
    // This should be enough for typical radar displays
    this.maxVertices = 2000000;

    // Current vertex count
    this.vertexCount = 0;

    // Render all spokes - don't skip any for full resolution
    this.spokeStep = 1;

    // Throttle updates - minimum ms between full rebuilds
    this.minUpdateInterval = 100; // 10 FPS max for vertex rebuilding (CPU intensive)
    this.lastUpdateTime = 0;
    this.pendingUpdate = false;

    // Pre-compute polar to cartesian lookup tables
    this.#precomputeCoordinates();

    // Initialize rendering resources
    this.#initBuffers();
    this.#initPipeline();

    // Legend color table (RGBA for each value 0-255)
    this.legendColors = new Float32Array(256 * 4);
    this.#initDefaultLegend();
  }

  /**
   * Pre-compute polar to cartesian coordinates for all spokes and range bins
   */
  #precomputeCoordinates() {
    const { spokesPerRevolution, maxSpokeLen } = this;

    // Arrays for x, y coordinates at each (spoke, range) position
    // x[spoke * maxSpokeLen + range], y[spoke * maxSpokeLen + range]
    this.cosTable = new Float32Array(spokesPerRevolution);
    this.sinTable = new Float32Array(spokesPerRevolution);

    for (let spoke = 0; spoke < spokesPerRevolution; spoke++) {
      // Angle in radians - 0 is straight ahead (up/north), clockwise
      // Standard radar convention: angle 0 = north, increasing clockwise
      const angle = (spoke / spokesPerRevolution) * 2 * Math.PI;

      // Marine radar: 0° = North (up), 90° = East (right)
      // Math coords: 0° = East (right), 90° = North (up)
      // Convert: screen_x = sin(angle), screen_y = -cos(angle) for north-up
      this.cosTable[spoke] = Math.sin(angle);  // x component
      this.sinTable[spoke] = -Math.cos(angle); // y component (negative for screen coords)
    }

    console.log(`SpokeRenderer: Pre-computed coordinates for ${spokesPerRevolution} spokes (step=${this.spokeStep}, effective=${Math.floor(spokesPerRevolution / this.spokeStep)})`);
  }

  /**
   * Initialize default legend (green gradient like traditional radar)
   */
  #initDefaultLegend() {
    for (let i = 0; i < 256; i++) {
      // Value 0 = transparent
      if (i === 0) {
        this.legendColors[i * 4] = 0;     // R
        this.legendColors[i * 4 + 1] = 0; // G
        this.legendColors[i * 4 + 2] = 0; // B
        this.legendColors[i * 4 + 3] = 0; // A
        continue;
      }

      // Values 1-255: intensity gradient
      // Low values: dark blue
      // Mid values: green
      // High values: red/yellow
      const normalized = i / 255;

      let r, g, b;
      if (normalized < 0.33) {
        // Blue to cyan
        const t = normalized / 0.33;
        r = 0;
        g = t * 0.5;
        b = 0.3 + t * 0.4;
      } else if (normalized < 0.66) {
        // Cyan to green to yellow
        const t = (normalized - 0.33) / 0.33;
        r = t * 0.8;
        g = 0.5 + t * 0.5;
        b = 0.7 - t * 0.7;
      } else {
        // Yellow to red
        const t = (normalized - 0.66) / 0.34;
        r = 0.8 + t * 0.2;
        g = 1.0 - t * 0.5;
        b = 0;
      }

      this.legendColors[i * 4] = r;
      this.legendColors[i * 4 + 1] = g;
      this.legendColors[i * 4 + 2] = b;
      this.legendColors[i * 4 + 3] = 1.0; // Full opacity for non-zero values
    }
  }

  /**
   * Set legend from external source (RGBA array per value)
   * @param {Array} legend - Array of [r, g, b, a] for each value 0-255
   */
  setLegend(legend) {
    for (let i = 0; i < Math.min(legend.length, 256); i++) {
      this.legendColors[i * 4] = legend[i][0] / 255;
      this.legendColors[i * 4 + 1] = legend[i][1] / 255;
      this.legendColors[i * 4 + 2] = legend[i][2] / 255;
      this.legendColors[i * 4 + 3] = legend[i][3] / 255;
    }
  }

  #initBuffers() {
    // Vertex buffer: position (x, y) + color (r, g, b, a) = 6 floats per vertex
    // Use mappable buffer for CPU updates
    this.vertexBuffer = this.device.createBuffer({
      size: this.maxVertices * 6 * 4, // 6 floats * 4 bytes
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
    });

    // Uniform buffer for transformation matrix
    this.uniformBuffer = this.device.createBuffer({
      size: 64, // 4x4 matrix
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    // CPU-side vertex data for building each frame
    this.vertexData = new Float32Array(this.maxVertices * 6);
  }

  #initPipeline() {
    const shaderModule = this.device.createShaderModule({
      code: spokeShaderCode,
    });

    const bindGroupLayout = this.device.createBindGroupLayout({
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.VERTEX,
          buffer: { type: "uniform" },
        },
      ],
    });

    this.bindGroup = this.device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: this.uniformBuffer } },
      ],
    });

    this.pipeline = this.device.createRenderPipeline({
      layout: this.device.createPipelineLayout({
        bindGroupLayouts: [bindGroupLayout],
      }),
      vertex: {
        module: shaderModule,
        entryPoint: "vertexMain",
        buffers: [
          {
            arrayStride: 24, // 6 floats * 4 bytes
            attributes: [
              { shaderLocation: 0, offset: 0, format: "float32x2" },  // position
              { shaderLocation: 1, offset: 8, format: "float32x4" },  // color
            ],
          },
        ],
      },
      fragment: {
        module: shaderModule,
        entryPoint: "fragmentMain",
        targets: [{
          format: this.canvasFormat,
          blend: {
            color: {
              srcFactor: "src-alpha",
              dstFactor: "one-minus-src-alpha",
              operation: "add",
            },
            alpha: {
              srcFactor: "one",
              dstFactor: "one-minus-src-alpha",
              operation: "add",
            },
          },
        }],
      },
      primitive: {
        topology: "line-list",
      },
    });
  }

  /**
   * Update the transformation matrix
   * @param {Float32Array} matrix - 4x4 transformation matrix (column-major)
   */
  setTransform(matrix) {
    this.device.queue.writeBuffer(this.uniformBuffer, 0, matrix);
  }

  /**
   * Build vertex buffer from spoke data
   *
   * Renders spokes as line segments connecting adjacent angular positions.
   * Each range bin creates a line from spoke[i] to spoke[i+1].
   *
   * @param {Uint8Array} spokeData - Raw spoke data (spokesPerRev * maxSpokeLen bytes)
   * @param {number} threshold - Values below this are not rendered
   */
  updateFromSpokeData(spokeData, threshold = 5) {
    // Throttle updates to avoid overwhelming the system
    const now = performance.now();
    if (now - this.lastUpdateTime < this.minUpdateInterval) {
      this.pendingUpdate = true;
      return; // Skip this update, render with existing vertices
    }
    this.lastUpdateTime = now;
    this.pendingUpdate = false;

    const { spokesPerRevolution, maxSpokeLen, cosTable, sinTable, legendColors, spokeStep } = this;

    let vertexIndex = 0;
    let lineCount = 0;

    // Iterate through spokes (with stepping for performance)
    for (let spoke = 0; spoke < spokesPerRevolution; spoke += spokeStep) {
      const nextSpoke = (spoke + spokeStep) % spokesPerRevolution;
      const spokeOffset = spoke * maxSpokeLen;

      // Get pre-computed sin/cos for both spoke angles
      const cos0 = cosTable[spoke];
      const sin0 = sinTable[spoke];
      const cos1 = cosTable[nextSpoke];
      const sin1 = sinTable[nextSpoke];

      // Iterate through range bins
      for (let range = 0; range < maxSpokeLen; range++) {
        // Get value at current spoke and range
        const value = spokeData[spokeOffset + range];

        // Skip values below threshold
        if (value < threshold) continue;

        // Normalize radius to [-1, 1] range
        const r = (range + 0.5) / maxSpokeLen;

        // Calculate Cartesian coordinates for line endpoints
        const x0 = cos0 * r;
        const y0 = sin0 * r;
        const x1 = cos1 * r;
        const y1 = sin1 * r;

        // Look up color from legend
        const colorIndex = value * 4;
        const cr = legendColors[colorIndex];
        const cg = legendColors[colorIndex + 1];
        const cb = legendColors[colorIndex + 2];
        const ca = legendColors[colorIndex + 3];

        // Skip fully transparent
        if (ca === 0) continue;

        // Add line vertices (2 vertices per line)
        // Vertex 0: start of line
        this.vertexData[vertexIndex++] = x0;
        this.vertexData[vertexIndex++] = y0;
        this.vertexData[vertexIndex++] = cr;
        this.vertexData[vertexIndex++] = cg;
        this.vertexData[vertexIndex++] = cb;
        this.vertexData[vertexIndex++] = ca;

        // Vertex 1: end of line
        this.vertexData[vertexIndex++] = x1;
        this.vertexData[vertexIndex++] = y1;
        this.vertexData[vertexIndex++] = cr;
        this.vertexData[vertexIndex++] = cg;
        this.vertexData[vertexIndex++] = cb;
        this.vertexData[vertexIndex++] = ca;

        lineCount++;

        // Safety check
        if (vertexIndex >= this.vertexData.length - 12) {
          console.warn(`SpokeRenderer: Vertex buffer full at ${lineCount} lines`);
          break;
        }
      }

      if (vertexIndex >= this.vertexData.length - 12) break;
    }

    this.vertexCount = lineCount * 2;

    // Upload to GPU
    if (this.vertexCount > 0) {
      this.device.queue.writeBuffer(
        this.vertexBuffer,
        0,
        this.vertexData.buffer,
        0,
        this.vertexCount * 6 * 4
      );
    }

    // Debug logging
    if (!this._lastUpdateLog || now - this._lastUpdateLog > 2000) {
      this._lastUpdateLog = now;
      console.log(`SpokeRenderer: ${lineCount} lines, ${this.vertexCount} vertices, threshold=${threshold}, step=${spokeStep}`);
    }
  }

  /**
   * Render the spoke lines
   * @param {GPUCommandEncoder} encoder - Command encoder to use (optional)
   * @param {GPUTextureView} targetView - Render target view (optional)
   * @param {boolean} clear - Whether to clear the target first
   */
  render(encoder, targetView, clear = true) {
    if (this.vertexCount === 0) return;

    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: targetView,
          clearValue: { r: 0, g: 0, b: 0, a: 0 },
          loadOp: clear ? "clear" : "load",
          storeOp: "store",
        },
      ],
    });

    pass.setPipeline(this.pipeline);
    pass.setBindGroup(0, this.bindGroup);
    pass.setVertexBuffer(0, this.vertexBuffer);
    pass.draw(this.vertexCount);
    pass.end();
  }

  /**
   * Standalone render (creates own encoder)
   */
  renderStandalone() {
    if (this.vertexCount === 0) {
      if (!this._lastNoDataLog || performance.now() - this._lastNoDataLog > 2000) {
        this._lastNoDataLog = performance.now();
        console.log('SpokeRenderer: No vertices to render');
      }
      return;
    }

    const encoder = this.device.createCommandEncoder();
    const targetView = this.context.getCurrentTexture().createView();

    this.render(encoder, targetView, true);

    this.device.queue.submit([encoder.finish()]);
  }
}

/**
 * Simple line shader for spoke rendering
 */
const spokeShaderCode = `
struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u_transform: mat4x4<f32>;

@vertex
fn vertexMain(
  @location(0) pos: vec2<f32>,
  @location(1) color: vec4<f32>
) -> VertexOutput {
  var output: VertexOutput;
  output.position = u_transform * vec4<f32>(pos, 0.0, 1.0);
  output.color = color;
  return output;
}

@fragment
fn fragmentMain(@location(0) color: vec4<f32>) -> @location(0) vec4<f32> {
  // Output with alpha blending
  return color;
}
`;
