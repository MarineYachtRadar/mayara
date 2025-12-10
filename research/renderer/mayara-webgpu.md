# Mayara WebGPU Renderer Analysis (render_webgpu.js)

This document analyzes the WebGPU-based renderer from Mayara, which uses texture-based polar-to-cartesian conversion with the modern WebGPU API.

## Source File
- **Location**: `mayara-signalk-wasm/public/render_webgpu.js`
- **Lines**: ~490
- **API**: WebGPU (navigator.gpu)

---

## 1. Rendering Technique Overview

### Approach: WebGPU Texture-Based Rendering

1. Store radar data in a 2D texture (polar: angle × radius)
2. Draw fullscreen quad (4 vertices, TRIANGLE_STRIP)
3. Fragment shader converts cartesian to polar coordinates
4. Sample radar texture and color table for final color
5. **Overlay canvas** for range rings on top of radar

### Key Features (Current Implementation)
- **Async initialization**: WebGPU requires await for device setup
- **Bind groups**: Resources bound via bind groups
- **WGSL shaders**: WebGPU Shading Language
- **Neighbor filling**: Enhances adjacent spokes for solid appearance
- **Overlay canvas**: Separate canvas for range rings on top of radar
- **Range change clearing**: Clears spoke data when range changes

---

## 2. Canvas Architecture

### Three-Layer Canvas Stack

```
z-index 3: myr_canvas_overlay  (range rings - green, on TOP)
z-index 2: myr_canvas_webgpu   (radar spokes - WebGPU rendered)
z-index 1: myr_canvas_background (title, no-transmit zones)
```

```javascript
constructor(canvas_dom, canvas_background_dom, drawBackground) {
    this.dom = canvas_dom;  // WebGPU canvas
    this.background_dom = canvas_background_dom;
    this.background_ctx = this.background_dom.getContext("2d");
    // Overlay canvas for range rings (on top of radar)
    this.overlay_dom = document.getElementById("myr_canvas_overlay");
    this.overlay_ctx = this.overlay_dom ? this.overlay_dom.getContext("2d") : null;
}
```

---

## 3. Async Initialization

### Constructor Pattern

```javascript
constructor(canvas_dom, canvas_background_dom, drawBackground) {
    this.ready = false;
    this.pendingLegend = null;
    this.pendingSpokes = null;
    this.actual_range = 0;

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

    this.ready = true;
    // Apply any pending calls
    if (this.pendingSpokes) { ... }
    if (this.pendingLegend) { ... }
}
```

### Pending Call Pattern
Since setSpokes/setLegend may be called before WebGPU is ready:

```javascript
setSpokes(spokesPerRevolution, max_spoke_len) {
    if (!this.ready) {
        this.pendingSpokes = { spokesPerRevolution, max_spoke_len };
        this.spokesPerRevolution = spokesPerRevolution;
        this.max_spoke_len = max_spoke_len;
        this.data = new Uint8Array(spokesPerRevolution * max_spoke_len);
        return;
    }
    // ... normal setup
}
```

---

## 4. Resource Creation

### Polar Data Texture

```javascript
this.polarTexture = this.device.createTexture({
    size: [max_spoke_len, spokesPerRevolution],
    format: "r8unorm",  // 8-bit unsigned normalized [0,1]
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
});
```

### Color Table Texture

```javascript
this.colorTexture = this.device.createTexture({
    size: [256, 1],
    format: "rgba8unorm",
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
});

// Upload color data
const colorTableData = new Uint8Array(256 * 4);
for (let i = 0; i < l.length; i++) {
    colorTableData[i * 4] = l[i][0];      // R
    colorTableData[i * 4 + 1] = l[i][1];  // G
    colorTableData[i * 4 + 2] = l[i][2];  // B
    colorTableData[i * 4 + 3] = l[i][3];  // A
}
```

### Sampler

```javascript
this.sampler = this.device.createSampler({
    magFilter: "linear",
    minFilter: "linear",
    addressModeU: "clamp-to-edge",
    addressModeV: "repeat",  // Wrap around for angles
});
```

### Uniform Buffer

```javascript
this.uniformBuffer = this.device.createBuffer({
    size: 32,  // scaleX, scaleY, spokesPerRev, maxSpokeLen + padding
    usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
});
```

---

## 5. Bind Group Layout

```javascript
this.bindGroupLayout = this.device.createBindGroupLayout({
    entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT,
          texture: { sampleType: "float" } },           // polar data
        { binding: 1, visibility: GPUShaderStage.FRAGMENT,
          texture: { sampleType: "float" } },           // color table
        { binding: 2, visibility: GPUShaderStage.FRAGMENT,
          sampler: { type: "filtering" } },             // sampler
        { binding: 3, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: "uniform" } },                // uniforms
    ],
});

this.bindGroup = this.device.createBindGroup({
    layout: this.bindGroupLayout,
    entries: [
        { binding: 0, resource: this.polarTexture.createView() },
        { binding: 1, resource: this.colorTexture.createView() },
        { binding: 2, resource: this.sampler },
        { binding: 3, resource: { buffer: this.uniformBuffer } },
    ],
});
```

---

## 6. Render Pipeline

```javascript
this.renderPipeline = this.device.createRenderPipeline({
    layout: this.device.createPipelineLayout({ bindGroupLayouts: [this.bindGroupLayout] }),
    vertex: {
        module: shaderModule,
        entryPoint: "vertexMain",
        buffers: [{
            arrayStride: 16,  // 4 floats × 4 bytes
            attributes: [
                { shaderLocation: 0, offset: 0, format: "float32x2" },  // position
                { shaderLocation: 1, offset: 8, format: "float32x2" },  // texCoord
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
```

---

## 7. WGSL Shaders

### Uniform Structure

```wgsl
struct Uniforms {
  scaleX: f32,
  scaleY: f32,
  spokesPerRev: f32,
  maxSpokeLen: f32,
}

@group(0) @binding(3) var<uniform> uniforms: Uniforms;
```

### Vertex Shader

```wgsl
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) texCoord: vec2<f32>,
}

@vertex
fn vertexMain(@location(0) pos: vec2<f32>, @location(1) texCoord: vec2<f32>) -> VertexOutput {
    var output: VertexOutput;
    let scaledPos = vec2<f32>(pos.x * uniforms.scaleX, pos.y * uniforms.scaleY);
    output.position = vec4<f32>(scaledPos, 0.0, 1.0);
    output.texCoord = texCoord;
    return output;
}
```

### Fragment Shader

```wgsl
@group(0) @binding(0) var polarData: texture_2d<f32>;
@group(0) @binding(1) var colorTable: texture_2d<f32>;
@group(0) @binding(2) var texSampler: sampler;

const PI: f32 = 3.14159265359;
const TWO_PI: f32 = 6.28318530718;

@fragment
fn fragmentMain(@location(0) texCoord: vec2<f32>) -> @location(0) vec4<f32> {
    // Convert cartesian (texCoord) to polar for sampling radar data
    let centered = texCoord - vec2<f32>(0.5, 0.5);

    // Calculate radius (0 at center, 1 at edge)
    let r = length(centered) * 2.0;

    // Calculate angle - clockwise from top (bow)
    // atan2(x, y) gives clockwise angle from Y-axis
    var theta = atan2(centered.x, centered.y);
    if (theta < 0.0) {
        theta = theta + TWO_PI;
    }

    // Normalize to [0, 1] for texture V coordinate
    let normalizedTheta = theta / TWO_PI;

    // Sample polar data
    // U = radius [0,1], V = angle [0,1]
    // V: 0=bow, 0.25=starboard, 0.5=stern, 0.75=port
    let radarValue = textureSample(polarData, texSampler, vec2<f32>(r, normalizedTheta)).r;

    // Look up color from table
    let color = textureSample(colorTable, texSampler, vec2<f32>(radarValue, 0.0));

    // Mask pixels outside radar circle
    let insideCircle = step(r, 1.0);
    let hasData = step(0.004, radarValue);  // ~1/255 threshold
    let alpha = hasData * color.a * insideCircle;

    return vec4<f32>(color.rgb * insideCircle, alpha);
}
```

### Radar Angle Convention

```
Radar convention (from Furuno protobuf):
- angle 0 = bow (top on screen)
- angle increases clockwise: bow -> starboard -> stern -> port -> bow

Screen coordinates (after centering):
- Top (bow):      centered = (0, +0.5)   -> theta = 0
- Right (stbd):   centered = (+0.5, 0)   -> theta = PI/2
- Bottom (stern): centered = (0, -0.5)   -> theta = PI
- Left (port):    centered = (-0.5, 0)   -> theta = 3PI/2
```

---

## 8. Neighbor Filling (Solid Radar Display)

The renderer enhances adjacent spokes for a more solid appearance:

```javascript
drawSpoke(spoke) {
    if (!this.data) return;

    // Clear data when range changes
    if (this.actual_range != spoke.range) {
        this.actual_range = spoke.range;
        this.data.fill(0);
        this.redrawCanvas();
    }

    const spokeLen = spoke.data.length;
    const maxLen = this.max_spoke_len;
    const spokes = this.spokesPerRevolution;

    // Calculate neighbor spoke offsets (±4 spokes with wrap-around)
    const blendFactors = [0.9, 0.75, 0.55, 0.35]; // Falloff for each distance

    const neighborOffsets = [];
    for (let d = 1; d <= 4; d++) {
        const prev = (spoke.angle + spokes - d) % spokes;
        const next = (spoke.angle + d) % spokes;
        neighborOffsets.push({
            prevOffset: prev * maxLen,
            nextOffset: next * maxLen,
            blend: blendFactors[d - 1]
        });
    }

    let offset = spoke.angle * this.max_spoke_len;

    for (let i = 0; i < spokeLen; i++) {
        const val = spoke.data[i];
        // Write current spoke at full value
        this.data[offset + i] = val;

        // Enhance neighbors if this pixel has signal
        if (val > 1) {
            for (const n of neighborOffsets) {
                const blendVal = Math.floor(val * n.blend);
                if (this.data[n.prevOffset + i] < blendVal) {
                    this.data[n.prevOffset + i] = blendVal;
                }
                if (this.data[n.nextOffset + i] < blendVal) {
                    this.data[n.nextOffset + i] = blendVal;
                }
            }
        }
    }
}
```

### Blend Factor Falloff

| Distance | Factor | Effect |
|----------|--------|--------|
| ±1 spoke | 0.90 | 90% intensity |
| ±2 spokes | 0.75 | 75% intensity |
| ±3 spokes | 0.55 | 55% intensity |
| ±4 spokes | 0.35 | 35% intensity |

---

## 9. Overlay Canvas (Range Rings)

Range rings are drawn on a separate overlay canvas on TOP of radar:

```javascript
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

        // Draw range labels at 45 degrees (upper right)
        if (range) {
            const text = formatRangeValue(is_metric(range), (range * i) / 4);
            const labelX = this.center_x + (radius * 0.707);
            const labelY = this.center_y - (radius * 0.707);
            ctx.fillText(text, labelX + 5, labelY - 5);
        }
    }
}
```

---

## 10. Range Change Handling

When range changes, old spoke data is cleared:

```javascript
setRange(range) {
    this.range = range;
    // Clear spoke data when range changes - old data is no longer valid
    if (this.data) {
        this.data.fill(0);
    }
    this.redrawCanvas();
}

drawSpoke(spoke) {
    // Also check in drawSpoke when range comes from spoke data
    if (this.actual_range != spoke.range) {
        this.actual_range = spoke.range;
        this.data.fill(0);  // Clear old data
        this.redrawCanvas();
    }
    // ...
}
```

---

## 11. Debug Display

Debug information is drawn on the background canvas:

```javascript
#updateUniforms() {
    const range = this.range || this.actual_range || 1500;
    const scale = (1.0 * this.actual_range) / range;

    const scaleX = scale * ((2 * this.beam_length) / this.width);
    const scaleY = scale * ((2 * this.beam_length) / this.height);

    // Pack uniforms
    const uniforms = new Float32Array([
        scaleX, scaleY,
        this.spokesPerRevolution || 2048,
        this.max_spoke_len || 512,
        0, 0, 0, 0  // padding to 32 bytes
    ]);

    this.device.queue.writeBuffer(this.uniformBuffer, 0, uniforms);

    // Debug labels with units
    this.background_ctx.fillStyle = "lightgreen";
    this.background_ctx.fillText("Beam length: " + this.beam_length + " px", 5, 40);
    this.background_ctx.fillText("Display range: " + formatRangeValue(is_metric(range), range), 5, 60);
    this.background_ctx.fillText("Radar range: " + formatRangeValue(is_metric(this.actual_range), this.actual_range), 5, 80);
    this.background_ctx.fillText("Spoke length: " + (this.max_spoke_len || 0) + " px", 5, 100);
}
```

---

## 12. Rendering Loop

```javascript
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
```

---

## 13. Memory Layout

### Texture Memory

| Texture | Format | Size | Bytes |
|---------|--------|------|-------|
| Polar data | r8unorm | max_spoke_len × spokesPerRev | ~2 MB (512×8192) |
| Color table | rgba8unorm | 256 × 1 | 1 KB |
| **Total** | | | ~2 MB |

### Buffer Memory

| Buffer | Size | Purpose |
|--------|------|---------|
| Uniform | 32 bytes | scaleX, scaleY, spokesPerRev, maxSpokeLen |
| Vertex | 64 bytes | Fullscreen quad (4 vertices × 4 floats) |
| **Total** | ~100 bytes | |

---

## 14. Pros and Cons

### Advantages
1. **Modern API**: WebGPU is the future of web graphics
2. **Better performance potential**: More explicit control over GPU
3. **Neighbor filling**: Creates solid radar display without manual blending
4. **Overlay canvas**: Range rings visible on top of radar spokes
5. **Range-aware clearing**: Properly handles range changes

### Disadvantages
1. **Browser support**: Requires modern browsers (Chrome 113+, Edge, Firefox Nightly)
2. **Complexity**: More boilerplate than WebGL
3. **Async setup**: Requires careful handling of initialization
4. **No fallback**: Must detect and use WebGL if unavailable

---

## 15. WebGPU vs WebGL Comparison

| Aspect | WebGPU | WebGL2 |
|--------|--------|--------|
| API style | Modern, explicit | Legacy OpenGL |
| Initialization | Async (await) | Sync |
| Resource binding | Bind groups | Individual uniforms |
| Shader language | WGSL | GLSL ES |
| Command submission | Command encoder | Immediate mode |
| Browser support | Chrome 113+, Edge | Universal |
| Texture upload | queue.writeTexture | texImage2D |

---

## 16. Usage in viewer.js

```javascript
// Detection and fallback
try {
    if (draw == "webgpu") {
        renderer = new render_webgpu(canvas, background, drawBackground);
    }
} catch (e) {
    console.log("WebGPU not available, falling back to WebGL");
    renderer = new render_webgl(canvas, background, drawBackground);
}

// Default behavior: WebGPU if available
if (navigator.gpu) {
    renderer = new render_webgpu(canvas, background, drawBackground);
} else {
    renderer = new render_webgl(canvas, background, drawBackground);
}
```

---

## 17. Key Implementation Details

### Furuno DRS4D-NXT Specifics
- **6-bit pixel values**: Spoke data uses values 0-63 (scaled in color table)
- **8192 spokes/revolution**: High spoke count for smooth display
- **512 samples/spoke**: Range resolution per spoke
- **Range table**: Non-sequential wire indices for Furuno protocol

### Color Table
- 256 entries for full 8-bit lookup
- First 64 entries used for actual radar data (6-bit)
- Index 0 = transparent (background)
- Higher values = stronger returns (typically yellow/red)
